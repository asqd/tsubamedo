//! OAuth 2.0 for requests with `[auth] type = "oauth2"`: the token is got before the request
//! and sent as `Authorization: Bearer`, kept until it expires and renewed with the refresh
//! token when the server gave one.
//!
//! - Authorization code: the sign-in page opens in the browser, the provider sends the browser
//!   back to `http://127.0.0.1:<port>`, where the app waits for the code (RFC 8252). PKCE
//!   (S256) is on unless switched off.
//! - Client credentials and password: one request to the token endpoint.
//!
//! Tokens are kept by what makes them (grant, addresses, client, scope, user). The session
//! stores them in the app's data folder, never in the collection; the CLI for one run.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

use crate::http::{self, SendOptions};
use crate::model::{ClientAuth, Grant, OAuth2, RequestOptions};
use crate::prepare::{Prepared, parse_url};
use crate::{Error, Result};

/// How long the app waits for the sign-in in the browser.
const SIGN_IN_WAIT: Duration = Duration::from_secs(300);
/// A token this close to its end (seconds) is renewed before use.
const EXPIRY_MARGIN: i64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds; `None` when the server did not say.
    pub expires_at: Option<i64>,
    pub scope: Option<String>,
    /// Unix seconds.
    pub obtained_at: i64,
}

impl Token {
    pub fn usable(&self) -> bool {
        self.expires_at.is_none_or(|e| e - EXPIRY_MARGIN > now())
    }

    pub fn header(&self) -> String {
        format!("Bearer {}", self.access_token)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Tokens {
    tokens: BTreeMap<String, Token>,
    #[serde(skip)]
    changed: bool,
}

/// One store for requests going side by side: the second waits while the first gets the token.
pub type SharedTokens = Arc<tokio::sync::Mutex<Tokens>>;

impl Tokens {
    pub fn shared(self) -> SharedTokens {
        Arc::new(tokio::sync::Mutex::new(self))
    }

    pub fn get(&self, o: &OAuth2) -> Option<&Token> {
        self.tokens.get(&key(o))
    }

    pub fn forget(&mut self, o: &OAuth2) -> bool {
        let gone = self.tokens.remove(&key(o)).is_some();
        self.changed |= gone;
        gone
    }

    pub fn clear(&mut self) {
        self.changed |= !self.tokens.is_empty();
        self.tokens.clear();
    }

    /// Whether it changed since the last call: the owner saves it then.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    fn put(&mut self, o: &OAuth2, token: Token) {
        self.tokens.insert(key(o), token);
        self.changed = true;
    }
}

/// What makes a token: settings that would give another one. Hashed, so the secret and the
/// password do not lie next to the token in the file.
fn key(o: &OAuth2) -> String {
    let all = [o.grant.as_str(), &o.auth_url, &o.token_url, &o.client_id, &o.client_secret, &o.scope, &o.audience, &o.username, &o.password].join("\n");
    ring::digest::digest(&ring::digest::SHA256, all.as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The kept token while it lasts.
    Cached,
    /// The server refused the kept one: the refresh token, or a new one.
    Renew,
    /// A new one, whatever is kept.
    New,
}

pub struct Got {
    pub token: Token,
    /// Got just now, not taken from the store.
    pub fresh: bool,
    /// For the log under the response: what was done to get it.
    pub note: Option<String>,
}

/// The token for these settings (variables already substituted).
pub async fn token(o: &OAuth2, store: &SharedTokens, options: &SendOptions, mode: Mode) -> Result<Got> {
    check(o)?;
    let mut store = store.lock().await;
    let old = store.get(o).cloned();
    if mode == Mode::Cached
        && let Some(t) = old.as_ref().filter(|t| t.usable())
    {
        return Ok(Got { token: t.clone(), fresh: false, note: None });
    }
    let options = token_options(options);
    let mut note = None;
    if mode != Mode::New
        && let Some(refresh) = old.as_ref().and_then(|t| t.refresh_token.clone())
    {
        match request_token(o, &[("grant_type", "refresh_token"), ("refresh_token", &refresh)], &options).await {
            Ok(mut t) => {
                // Servers that do not rotate refresh tokens leave the old one valid.
                t.refresh_token = t.refresh_token.or(Some(refresh));
                store.put(o, t.clone());
                let note = Some(format!("OAuth 2.0: токен обновлён по refresh token{}", lasts(&t)));
                return Ok(Got { token: t, fresh: true, note });
            }
            Err(e) => note = Some(format!("OAuth 2.0: refresh token не подошёл ({}), токен получен заново", reason(&e))),
        }
    }
    let t = grant(o, &options).await?;
    store.put(o, t.clone());
    let note = note.unwrap_or_else(|| format!("OAuth 2.0: получен новый токен ({}){}", describe(o.grant), lasts(&t)));
    Ok(Got { token: t, fresh: true, note: Some(note) })
}

fn describe(grant: Grant) -> &'static str {
    match grant {
        Grant::AuthorizationCode => "вход в браузере",
        Grant::ClientCredentials => "client credentials",
        Grant::Password => "по паролю",
    }
}

fn lasts(t: &Token) -> String {
    match t.expires_at {
        Some(at) => {
            let minutes = (at - t.obtained_at).max(0) / 60;
            if minutes > 0 { format!(", действует {minutes} мин") } else { format!(", действует {} с", (at - t.obtained_at).max(0)) }
        }
        None => String::new(),
    }
}

/// The message without the «OAuth 2.0:» it starts with.
fn reason(e: &Error) -> String {
    match e {
        Error::OAuth(m) => m.clone(),
        other => other.to_string(),
    }
}

fn check(o: &OAuth2) -> Result<()> {
    let missing = if o.token_url.trim().is_empty() {
        "адрес получения токена (token URL)"
    } else if o.client_id.trim().is_empty() {
        "client_id"
    } else if o.grant == Grant::AuthorizationCode && o.auth_url.trim().is_empty() {
        "адрес входа (authorization URL)"
    } else if o.grant == Grant::Password && o.username.trim().is_empty() {
        "имя пользователя"
    } else {
        return Ok(());
    };
    Err(Error::OAuth(format!("не задан {missing}")))
}

/// Without the cookie jar (the token endpoint has nothing to do with the API's session) and
/// never waiting forever.
fn token_options(options: &SendOptions) -> SendOptions {
    SendOptions {
        cookies: None,
        follow_redirects: true,
        timeout: Some(options.timeout.unwrap_or(Duration::from_secs(30))),
        ..options.clone()
    }
}

async fn grant(o: &OAuth2, options: &SendOptions) -> Result<Token> {
    let extra = [("scope", o.scope.as_str()), ("audience", o.audience.as_str())];
    match o.grant {
        Grant::ClientCredentials => {
            let mut params = vec![("grant_type", "client_credentials")];
            params.extend(extra);
            request_token(o, &params, options).await
        }
        Grant::Password => {
            let mut params = vec![("grant_type", "password"), ("username", o.username.as_str()), ("password", o.password.as_str())];
            params.extend(extra);
            request_token(o, &params, options).await
        }
        Grant::AuthorizationCode => {
            let signed = sign_in(o).await?;
            let mut params = vec![("grant_type", "authorization_code"), ("code", signed.code.as_str()), ("redirect_uri", signed.redirect_uri.as_str())];
            if let Some(verifier) = &signed.verifier {
                params.push(("code_verifier", verifier));
            }
            request_token(o, &params, options).await
        }
    }
}

/// A POST to the token endpoint; empty parameters are left out.
async fn request_token(o: &OAuth2, params: &[(&str, &str)], options: &SendOptions) -> Result<Token> {
    let url = parse_url(&o.token_url).map_err(|e| Error::OAuth(format!("адрес получения токена: {e}")))?;
    let mut form: Vec<(&str, &str)> = params.iter().filter(|(_, v)| !v.is_empty()).copied().collect();
    let mut headers = vec![
        ("Content-Type".to_string(), "application/x-www-form-urlencoded".to_string()),
        ("Accept".to_string(), "application/json".to_string()),
    ];
    let has_secret = !o.client_secret.is_empty();
    if o.client_auth == ClientAuth::Basic && has_secret {
        // RFC 6749 §2.3.1: both are form-encoded before Basic.
        let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
        let pair = format!("{}:{}", enc(&o.client_id), enc(&o.client_secret));
        headers.push(("Authorization".to_string(), format!("Basic {}", STANDARD.encode(pair))));
    } else {
        form.push(("client_id", &o.client_id));
        if has_secret {
            form.push(("client_secret", &o.client_secret));
        }
    }
    let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(form).finish();
    let prepared = Prepared {
        method: "POST".into(),
        url,
        headers,
        body: Bytes::from(body),
        options: RequestOptions::default(),
        oauth: None,
    };
    let response = http::send(&prepared, options).await.map_err(|e| Error::OAuth(format!("сервер токенов: {e}")))?;
    let bytes = response.body.head(1 << 20).map_err(|e| Error::OAuth(format!("ответ сервера токенов: {e}")))?;
    let text = String::from_utf8_lossy(&bytes);
    // JSON as the standard says; GitHub and old servers answer with a form.
    let fields: BTreeMap<String, Value> = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map)) => map.into_iter().collect(),
        _ => url::form_urlencoded::parse(text.trim().as_bytes()).map(|(k, v)| (k.into_owned(), Value::String(v.into_owned()))).collect(),
    };
    let field = |name: &str| match fields.get(name) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    let access_token = field("access_token");
    if response.status >= 400 || access_token.is_none() {
        let why = match (field("error"), field("error_description")) {
            (Some(e), Some(d)) => format!("{e} — {d}"),
            (Some(e), None) => e,
            _ if access_token.is_none() && response.status < 400 => "в ответе нет access_token".into(),
            _ => text.chars().take(200).collect::<String>().trim().to_string(),
        };
        return Err(Error::OAuth(format!("сервер токенов ответил {}: {why}", response.status)));
    }
    let obtained_at = now();
    Ok(Token {
        access_token: access_token.unwrap_or_default(),
        refresh_token: field("refresh_token"),
        expires_at: field("expires_in").and_then(|s| s.parse::<f64>().ok()).map(|s| obtained_at + s as i64),
        scope: field("scope"),
        obtained_at,
    })
}

struct SignedIn {
    code: String,
    redirect_uri: String,
    verifier: Option<String>,
}

/// Opens the sign-in page and waits for the browser to come back with the code.
async fn sign_in(o: &OAuth2) -> Result<SignedIn> {
    let (listener, redirect_uri) = listen(&o.redirect_uri).await?;
    let state = random();
    let verifier = o.pkce.then(random);
    let mut url = parse_url(&o.auth_url).map_err(|e| Error::OAuth(format!("адрес входа: {e}")))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code").append_pair("client_id", &o.client_id).append_pair("redirect_uri", &redirect_uri);
        for (name, value) in [("scope", &o.scope), ("audience", &o.audience)] {
            if !value.is_empty() {
                q.append_pair(name, value);
            }
        }
        q.append_pair("state", &state);
        if let Some(v) = &verifier {
            let challenge = URL_SAFE_NO_PAD.encode(ring::digest::digest(&ring::digest::SHA256, v.as_bytes()));
            q.append_pair("code_challenge", &challenge).append_pair("code_challenge_method", "S256");
        }
    }
    open_browser(url.as_str())?;
    let params = tokio::time::timeout(SIGN_IN_WAIT, wait_for_code(&listener, &state))
        .await
        .map_err(|_| Error::OAuth(format!("вход в браузере не завершён за {} минут", SIGN_IN_WAIT.as_secs() / 60)))??;
    if let Some(error) = params.get("error") {
        let detail = params.get("error_description").map(|d| format!(" — {d}")).unwrap_or_default();
        return Err(Error::OAuth(format!("вход отклонён: {error}{detail}")));
    }
    if params.get("state") != Some(&state) {
        return Err(Error::OAuth("в ответе входа чужой state — код пришёл не на этот вход".into()));
    }
    let code = params.get("code").cloned().unwrap_or_default();
    Ok(SignedIn { code, redirect_uri, verifier })
}

/// 32 random bytes, for `state` and the PKCE verifier (43 characters, as RFC 7636 wants).
fn random() -> String {
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 32];
    ring::rand::SystemRandom::new().fill(&mut bytes).expect("the system gives random bytes");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Where the browser comes back: the address given (on this machine only) or a free port.
async fn listen(redirect: &str) -> Result<(TcpListener, String)> {
    let redirect = redirect.trim();
    if redirect.is_empty() {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| Error::OAuth(format!("не открыть порт для возврата из браузера: {e}")))?;
        let port = listener.local_addr().map_err(|e| Error::OAuth(e.to_string()))?.port();
        return Ok((listener, format!("http://127.0.0.1:{port}/callback")));
    }
    let url = Url::parse(redirect).ok();
    let host = url.as_ref().and_then(Url::host_str).unwrap_or_default().to_string();
    let Some(url) = url.filter(|u| u.scheme() == "http" && matches!(host.as_str(), "127.0.0.1" | "localhost" | "[::1]")) else {
        return Err(Error::OAuth(format!(
            "адрес возврата {redirect}: код ловится только на http://127.0.0.1:порт или http://localhost:порт — оставьте поле пустым или впишите такой адрес и добавьте его у провайдера"
        )));
    };
    let port = url.port().unwrap_or(80);
    let bind = if host == "[::1]" { format!("[::1]:{port}") } else { format!("127.0.0.1:{port}") };
    let listener = TcpListener::bind(&bind).await.map_err(|e| Error::OAuth(format!("порт {port} для адреса возврата не открыть: {e}")))?;
    Ok((listener, redirect.to_string()))
}

/// The query of the first visit that brings `code` or `error`; the browser gets a page saying
/// how it went.
async fn wait_for_code(listener: &TcpListener, state: &str) -> Result<BTreeMap<String, String>> {
    loop {
        let (mut stream, _) = listener.accept().await.map_err(|e| Error::OAuth(format!("возврат из браузера: {e}")))?;
        let mut head = Vec::new();
        let mut chunk = [0u8; 2048];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 << 10 {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => head.extend_from_slice(&chunk[..n]),
            }
        }
        let head = String::from_utf8_lossy(&head);
        let target = head.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/");
        let params: BTreeMap<String, String> = Url::parse(&format!("http://localhost{target}"))
            .map(|u| u.query_pairs().into_owned().collect())
            .unwrap_or_default();
        if !params.contains_key("code") && !params.contains_key("error") {
            // The browser asking for a favicon, or someone else.
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            continue;
        }
        let ok = params.contains_key("code") && params.get("state").map(String::as_str) == Some(state);
        let (title, text) = if ok {
            ("Вход выполнен", "Можно закрыть вкладку и вернуться в TsubameDo.")
        } else {
            ("Вход не удался", "Что случилось — в TsubameDo, под запросом.")
        };
        let page = format!(
            "<!doctype html><meta charset=utf-8><title>TsubameDo</title><body style=\"font:16px -apple-system,system-ui,sans-serif;text-align:center;padding-top:20vh;color:#333\"><h2>{title}</h2><p>{text}</p></body>"
        );
        let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
        let _ = stream.write_all(reply.as_bytes()).await;
        let _ = stream.shutdown().await;
        return Ok(params);
    }
}

/// The system's browser; `$BROWSER` (a command) when set, as other tools do.
fn open_browser(url: &str) -> Result<()> {
    let mut command = match std::env::var("BROWSER").ok().filter(|b| !b.trim().is_empty()) {
        Some(browser) => {
            let mut words = browser.split_whitespace();
            let mut c = std::process::Command::new(words.next().unwrap_or_default());
            c.args(words);
            c
        }
        None if cfg!(target_os = "macos") => std::process::Command::new("open"),
        None if cfg!(windows) => {
            let mut c = std::process::Command::new("rundll32");
            c.arg("url.dll,FileProtocolHandler");
            c
        }
        None => std::process::Command::new("xdg-open"),
    };
    let mut child = command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error::OAuth(format!("не открыть браузер ({e}); адрес входа: {url}")))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}
