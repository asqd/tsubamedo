//! Cookies and OAuth 2.0 against a local server: a session cookie set on a redirect, tokens
//! got once and kept, renewed by the refresh token when the server stops taking them, and
//! the sign-in in the browser with PKCE (the "browser" is curl following the redirect).

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use base64::Engine;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tsubamedo_core::cookies::Jar;
use tsubamedo_core::http::SendOptions;
use tsubamedo_core::run::Runner;
use tsubamedo_core::{Auth, AwsV4, Body, ClientAuth, Grant, OAuth2, Request, Vars};

#[derive(Default)]
struct Server {
    /// Grants the token endpoint was asked for, in order.
    grants: Mutex<Vec<String>>,
    /// Access tokens the API takes now.
    valid: Mutex<Vec<String>>,
    /// PKCE challenge of the sign-in in progress.
    challenge: Mutex<String>,
    issued: Mutex<usize>,
}

fn reply(status: u16, headers: &[(&str, &str)], body: &str) -> hyper::Response<Full<Bytes>> {
    let mut b = hyper::Response::builder().status(status).header("content-type", "application/json");
    for (n, v) in headers {
        b = b.header(*n, *v);
    }
    b.body(Full::new(Bytes::from(body.to_string()))).unwrap()
}

async fn handle(s: Arc<Server>, req: hyper::Request<Incoming>) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path().to_string();
    let query: HashMap<String, String> = url::form_urlencoded::parse(req.uri().query().unwrap_or_default().as_bytes()).into_owned().collect();
    let header = |name: &str| req.headers().get(name).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned()).unwrap_or_default();
    let (cookie, authorization) = (header("cookie"), header("authorization"));
    if path.starts_with("/aws/") {
        return Ok(reply(200, &[], &format!("{}\n{authorization}", req.uri())));
    }
    let body = req.into_body().collect().await.unwrap().to_bytes();
    let form: HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
    Ok(match path.as_str() {
        // Login sets the session on the redirect, as web apps do.
        "/login" => reply(302, &[("set-cookie", "session=s1; Path=/; HttpOnly"), ("location", "/me")], ""),
        "/me" if cookie.contains("session=s1") => reply(200, &[], r#"{"user": "кот"}"#),
        "/me" => reply(401, &[], r#"{"error": "нет сессии"}"#),
        "/authorize" => {
            *s.challenge.lock().unwrap() = query.get("code_challenge").cloned().unwrap_or_default();
            let back = format!("{}?code=c-1&state={}", query["redirect_uri"], query["state"]);
            reply(302, &[("location", &back)], "")
        }
        "/token" => {
            let grant = form.get("grant_type").cloned().unwrap_or_default();
            s.grants.lock().unwrap().push(grant.clone());
            let basic = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode("app:s%26cret"));
            let client_ok = authorization == basic || (form.get("client_id").map(String::as_str) == Some("app") && !form.contains_key("client_secret"));
            let ok = client_ok
                && match grant.as_str() {
                    "client_credentials" => form.get("scope").map(String::as_str) == Some("orders"),
                    "refresh_token" => form.get("refresh_token").map(String::as_str) == Some("r-1"),
                    "authorization_code" => {
                        let verifier = form.get("code_verifier").cloned().unwrap_or_default();
                        let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
                        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
                        form.get("code").map(String::as_str) == Some("c-1") && challenge == *s.challenge.lock().unwrap()
                    }
                    _ => false,
                };
            if !ok {
                return Ok(reply(400, &[], r#"{"error": "invalid_grant", "error_description": "не тот клиент или код"}"#));
            }
            let mut issued = s.issued.lock().unwrap();
            *issued += 1;
            let token = format!("a-{issued}");
            s.valid.lock().unwrap().push(token.clone());
            reply(200, &[], &format!(r#"{{"access_token": "{token}", "token_type": "bearer", "expires_in": 3600, "refresh_token": "r-1"}}"#))
        }
        "/digest" => {
            const CHALLENGE: &str = r#"Digest realm="api", qop="auth", algorithm=SHA-256, nonce="n-1", opaque="o""#;
            let cnonce = authorization.split("cnonce=\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or_default();
            let want = tsubamedo_core::digest::answer(CHALLENGE, "GET", "/digest?x=1", b"", "кот", "мяу", cnonce).unwrap();
            if authorization == want {
                reply(200, &[], r#"{"ok": true}"#)
            } else {
                reply(401, &[("www-authenticate", "Basic realm=\"api\""), ("www-authenticate", CHALLENGE)], "")
            }
        }
        "/api" if s.valid.lock().unwrap().iter().any(|t| authorization == format!("Bearer {t}")) => reply(200, &[], r#"{"ok": true}"#),
        _ => reply(401, &[], r#"{"error": "invalid_token"}"#),
    })
}

async fn server() -> (SocketAddr, Arc<Server>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(Server::default());
    let s = state.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let s = s.clone();
            let service = service_fn(move |req| handle(s.clone(), req));
            tokio::spawn(hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream), service));
        }
    });
    (addr, state)
}

fn runner(options: SendOptions) -> Runner {
    Runner::new(None, None, Vars::new(), Vars::new(), options).unwrap()
}

fn get(url: String, auth: Auth) -> Request {
    Request { url, auth, ..Request::default() }
}

#[tokio::test(flavor = "multi_thread")]
async fn cookies_set_on_a_redirect_come_back() {
    let (addr, _) = server().await;
    let jar = Jar::default().shared();
    let mut r = runner(SendOptions { cookies: Some(jar.clone()), ..SendOptions::default() });
    let ex = r.send(None, &get(format!("http://{addr}/login"), Auth::None), Path::new(".")).await.pop().unwrap();
    assert_eq!(ex.result.as_ref().unwrap().status, 200, "the redirect to /me carried the session");

    let ex = r.send(None, &get(format!("http://{addr}/me"), Auth::None), Path::new(".")).await.pop().unwrap();
    assert_eq!(ex.result.as_ref().unwrap().status, 200);
    assert_eq!(ex.sent.unwrap().header("cookie"), Some("session=s1"), "what the jar added shows in what was sent");
    assert_eq!(jar.lock().unwrap().cookies()[0].domain, "127.0.0.1");

    // Switched off for the request: nothing sent, nothing kept.
    let mut off = get(format!("http://{addr}/me"), Auth::None);
    off.options.cookies = Some(false);
    let ex = r.send(None, &off, Path::new(".")).await.pop().unwrap();
    assert_eq!(ex.result.unwrap().status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn client_credentials_are_kept_and_renewed() {
    let (addr, s) = server().await;
    let auth = Auth::OAuth2(OAuth2 {
        grant: Grant::ClientCredentials,
        token_url: "{{base}}/token".into(),
        client_id: "app".into(),
        client_secret: "s&cret".into(),
        scope: "orders".into(),
        ..OAuth2::default()
    });
    let mut r = Runner::new(None, None, [("base".to_string(), format!("http://{addr}"))].into(), Vars::new(), SendOptions::default()).unwrap();
    let api = get("{{base}}/api".into(), auth.clone());
    let first = r.send(None, &api, Path::new(".")).await.pop().unwrap();
    assert_eq!(first.result.as_ref().unwrap().status, 200);
    assert_eq!(first.logs, ["OAuth 2.0: получен новый токен (client credentials), действует 60 мин"]);
    assert_eq!(first.sent.unwrap().header("authorization"), Some("Bearer a-1"));

    let second = r.send(None, &api, Path::new(".")).await.pop().unwrap();
    assert!(second.logs.is_empty() && second.passed());
    assert_eq!(*s.grants.lock().unwrap(), ["client_credentials"], "the token is kept");

    // The server forgets it: a 401, then the refresh token, then the same request again.
    s.valid.lock().unwrap().clear();
    let third = r.send(None, &api, Path::new(".")).await.pop().unwrap();
    assert_eq!(third.result.as_ref().unwrap().status, 200, "{:?}", third.logs);
    assert_eq!(third.sent.unwrap().header("authorization"), Some("Bearer a-2"));
    assert_eq!(*s.grants.lock().unwrap(), ["client_credentials", "refresh_token"]);

    // A wrong secret: the server's own words.
    let Auth::OAuth2(mut wrong) = auth else { unreachable!() };
    wrong.client_secret = "nope".into();
    let ex = r.send(None, &get("{{base}}/api".into(), Auth::OAuth2(wrong)), Path::new(".")).await.pop().unwrap();
    assert_eq!(ex.result.unwrap_err().to_string(), "OAuth 2.0: сервер токенов ответил 400: invalid_grant — не тот клиент или код");
    assert!(ex.sent.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn sign_in_in_the_browser_with_pkce() {
    let (addr, s) = server().await;
    // SAFETY: the only test in this binary that reads it, set before anything else runs it.
    unsafe { std::env::set_var("BROWSER", "curl -sL -o /dev/null") };
    let auth = Auth::OAuth2(OAuth2 {
        grant: Grant::AuthorizationCode,
        auth_url: format!("http://{addr}/authorize"),
        token_url: format!("http://{addr}/token"),
        client_id: "app".into(),
        client_auth: ClientAuth::Body,
        ..OAuth2::default()
    });
    let mut r = runner(SendOptions::default());
    let ex = r.send(None, &get(format!("http://{addr}/api"), auth), Path::new(".")).await.pop().unwrap();
    assert_eq!(ex.result.as_ref().map(|r| r.status).map_err(|e| e.to_string()), Ok(200), "{:?}", ex.logs);
    assert_eq!(ex.logs, ["OAuth 2.0: получен новый токен (вход в браузере), действует 60 мин"]);
    assert_eq!(*s.grants.lock().unwrap(), ["authorization_code"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn digest_answers_the_challenge() {
    let (addr, _) = server().await;
    let mut r = runner(SendOptions::default());
    let auth = Auth::Digest { username: "кот".into(), password: "мяу".into() };
    let ex = r.send(None, &get(format!("http://{addr}/digest?x=1"), auth), Path::new(".")).await.pop().unwrap();
    assert_eq!(ex.result.as_ref().unwrap().status, 200, "{:?}", ex.logs);
    assert!(ex.sent.unwrap().header("authorization").unwrap().starts_with(r#"Digest username="кот", realm="api", nonce="n-1", uri="/digest?x=1", algorithm=SHA-256"#));

    let wrong = Auth::Digest { username: "кот".into(), password: "гав".into() };
    let ex = r.send(None, &get(format!("http://{addr}/digest?x=1"), wrong), Path::new(".")).await.pop().unwrap();
    assert_eq!(ex.result.unwrap().status, 401, "a wrong password is the server's 401, sent once more only");
}

/// The signature against curl's own (`--aws-sigv4`), on what went: the query as the signer
/// rewrote it, the body, a session token. The path needs no encoding: curl before 8.8 does not
/// encode it twice for services other than S3, as AWS asks (the unit tests check that part).
/// Skipped with curl older than 8.7, the one it was checked with: 7.81 of Ubuntu 22.04 leaves
/// the `x-amz-*` headers given with `-H` out of its signature.
#[tokio::test(flavor = "multi_thread")]
async fn aws_signature_matches_curl() {
    let version = std::process::Command::new("curl").arg("--version").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let number: Vec<u32> = version.split_whitespace().nth(1).unwrap_or_default().split('.').map_while(|n| n.parse().ok()).collect();
    if number.as_slice() < [8, 7].as_slice() {
        eprintln!("curl {number:?} старше 8.7, сверка с --aws-sigv4 пропущена");
        return;
    }
    let (addr, _) = server().await;
    let mut r = runner(SendOptions::default());
    for service in ["s3", "execute-api"] {
        let aws = AwsV4 {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: "tok/1".into(),
            region: "eu-west-1".into(),
            service: service.into(),
        };
        let request = Request {
            method: "PUT".into(),
            url: format!("http://{addr}/aws/a-b_c/d.e~f?z=1&a=x y&a=0"),
            auth: Auth::AwsV4(aws),
            body: Body::Text { content: "мяу".into(), content_type: Some("text/plain".into()) },
            ..Request::default()
        };
        let ex = r.send(None, &request, Path::new(".")).await.pop().unwrap();
        let sent = ex.sent.unwrap();
        let got = ex.result.unwrap().body.head(1 << 16).unwrap();
        let (wire, ours) = std::str::from_utf8(&got).unwrap().split_once('\n').unwrap();
        assert_eq!(wire, "/aws/a-b_c/d.e~f?a=0&a=x%20y&z=1");
        assert_eq!(ours, sent.header("authorization").unwrap());

        let curl = std::process::Command::new("curl")
            .args(["-s", "-X", "PUT", "--data-binary", "мяу", "--aws-sigv4", &format!("aws:amz:eu-west-1:{service}")])
            .args(["--user", "AKIDEXAMPLE:wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"])
            .args(["-H", "Content-Type: text/plain", "-H", "X-Amz-Security-Token: tok/1"])
            .args(["-H", &format!("X-Amz-Date: {}", sent.header("x-amz-date").unwrap())])
            .args(sent.header("x-amz-content-sha256").map(|h| ["-H".to_string(), format!("X-Amz-Content-Sha256: {h}")]).into_iter().flatten())
            .arg(sent.url.as_str())
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&curl.stdout);
        let theirs = out.split_once('\n').map(|(_, a)| a).unwrap_or_default();
        assert_eq!(ours, theirs, "{service}");
    }
}
