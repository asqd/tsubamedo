//! Sending against a local hyper server: bodies, decompression, redirects, big answers, timeouts.

use std::convert::Infallible;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tsubamedo_core::http::{SendOptions, send};
use tsubamedo_core::prepare::prepare;
use tsubamedo_core::{Auth, Body, Error, Pair, Request, Vars};

const BIG: usize = 3 << 20;

async fn handle(req: hyper::Request<Incoming>) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let reply = |status: u16, headers: &[(&str, &str)], body: Vec<u8>| {
        let mut r = hyper::Response::builder().status(status);
        for (n, v) in headers {
            r = r.header(*n, *v);
        }
        Ok(r.body(Full::new(Bytes::from(body))).unwrap())
    };
    match req.uri().path() {
        "/echo" => {
            let mut text = format!("{} {}\n", req.method(), req.uri());
            for (n, v) in req.headers() {
                text.push_str(&format!("{n}: {}\n", v.to_str().unwrap()));
            }
            let body = req.into_body().collect().await.unwrap().to_bytes();
            text.push('\n');
            text.push_str(&String::from_utf8_lossy(&body));
            reply(200, &[("content-type", "text/plain")], text.into_bytes())
        }
        "/gzip" => {
            let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(r#"{"kot": "Матроскин"}"#.as_bytes()).unwrap();
            reply(200, &[("content-type", "application/json"), ("content-encoding", "gzip")], enc.finish().unwrap())
        }
        "/to-echo" => reply(302, &[("location", "/echo?from=redirect")], Vec::new()),
        "/see-other" => reply(303, &[("location", "/echo")], Vec::new()),
        "/loop" => reply(302, &[("location", "/loop")], Vec::new()),
        "/big" => reply(200, &[("content-type", "application/octet-stream")], vec![b'x'; BIG]),
        "/slow" => {
            tokio::time::sleep(Duration::from_secs(2)).await;
            reply(200, &[], Vec::new())
        }
        _ => reply(404, &[], b"nope".to_vec()),
    }
}

async fn server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream), service_fn(handle)));
        }
    });
    addr
}

fn vars(addr: SocketAddr) -> Vars {
    [("base".to_string(), format!("http://{addr}"))].into_iter().collect()
}

fn request(url: &str) -> Request {
    Request { url: url.into(), ..Request::default() }
}

async fn go(req: &Request, addr: SocketAddr, options: &SendOptions) -> Result<tsubamedo_core::http::Response, Error> {
    let prepared = prepare(req, &vars(addr), Path::new(".")).unwrap();
    send(&prepared, &options.for_request(&prepared)).await
}

fn text(r: &tsubamedo_core::http::Response) -> String {
    String::from_utf8(r.body.in_memory().unwrap().to_vec()).unwrap()
}

#[tokio::test]
async fn sends_method_headers_auth_and_body() {
    let addr = server().await;
    let req = Request {
        method: "PUT".into(),
        url: "{{base}}/echo".into(),
        query: vec![Pair::new("q", "кот")],
        headers: vec![Pair::new("X-Trace", "42")],
        auth: Auth::Bearer { token: "secret".into() },
        body: Body::Json { content: r#"{"a":1}"#.into() },
        ..Request::default()
    };
    let r = go(&req, addr, &SendOptions::default()).await.unwrap();
    assert_eq!((r.status, r.reason.as_str(), r.version.as_str()), (200, "OK", "HTTP/1.1"));
    let echo = text(&r);
    assert!(echo.starts_with("PUT /echo?q=%D0%BA%D0%BE%D1%82\n"), "{echo}");
    for line in ["x-trace: 42", "authorization: Bearer secret", "content-type: application/json", "content-length: 7", "user-agent: TsubameDo/"] {
        assert!(echo.contains(line), "{line} not in\n{echo}");
    }
    assert!(echo.ends_with("\n\n{\"a\":1}"), "{echo}");
    assert!(r.timings.total >= r.timings.wait);
    assert!(r.tls.is_none());
}

#[tokio::test]
async fn decompresses_gzip() {
    let addr = server().await;
    let r = go(&request("{{base}}/gzip"), addr, &SendOptions::default()).await.unwrap();
    assert_eq!(text(&r), r#"{"kot": "Матроскин"}"#);
    assert!(r.is_json());
    assert!(r.wire_size > 0 && r.wire_size != r.body.len());

    let raw = SendOptions { decompress: false, ..SendOptions::default() };
    let r = go(&request("{{base}}/gzip"), addr, &raw).await.unwrap();
    assert_eq!(&r.body.in_memory().unwrap()[..2], &[0x1f, 0x8b]);
}

#[tokio::test]
async fn follows_redirects_like_a_browser() {
    let addr = server().await;
    let r = go(&request("{{base}}/to-echo"), addr, &SendOptions::default()).await.unwrap();
    assert_eq!(r.status, 200);
    assert!(text(&r).starts_with("GET /echo?from=redirect\n"));
    assert_eq!(r.redirects.len(), 1);
    assert_eq!(r.redirects[0].status, 302);
    assert!(r.url.ends_with("/echo?from=redirect"));

    let post = Request { method: "POST".into(), url: "{{base}}/see-other".into(), body: Body::Json { content: "{}".into() }, ..Request::default() };
    let r = go(&post, addr, &SendOptions::default()).await.unwrap();
    let echo = text(&r);
    assert!(echo.starts_with("GET /echo\n") && !echo.contains("content-type"), "{echo}");

    let mut stay = request("{{base}}/to-echo");
    stay.options.follow_redirects = Some(false);
    assert_eq!(go(&stay, addr, &SendOptions::default()).await.unwrap().status, 302);

    let err = go(&request("{{base}}/loop"), addr, &SendOptions::default()).await.unwrap_err();
    assert!(matches!(err, Error::TooManyRedirects(10)), "{err}");
}

#[tokio::test]
async fn big_body_goes_to_a_file() {
    let addr = server().await;
    let dir = tempfile::tempdir().unwrap();
    let options = SendOptions { spill_threshold: 1 << 20, spill_dir: dir.path().to_path_buf(), ..SendOptions::default() };
    let r = go(&request("{{base}}/big"), addr, &options).await.unwrap();
    assert!(r.body.in_memory().is_none());
    assert_eq!(r.body.len(), BIG as u64);
    let path = r.body.path().unwrap().to_path_buf();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), BIG as u64);
    drop(r);
    assert!(!path.exists(), "the temporary file is removed with the response");
}

#[tokio::test]
async fn timeouts_and_connection_errors_are_readable() {
    let addr = server().await;
    let mut slow = request("{{base}}/slow");
    slow.options.timeout_ms = Some(200);
    let err = go(&slow, addr, &SendOptions::default()).await.unwrap_err();
    assert_eq!(err.to_string(), "ответ не пришёл за 200 мс");

    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
    let err = go(&request("{{base}}/x"), closed, &SendOptions::default()).await.unwrap_err();
    assert!(matches!(err, Error::Connect { .. }), "{err}");

    let err = go(&request("http://no-such-host.invalid/"), addr, &SendOptions::default()).await.unwrap_err();
    assert!(matches!(err, Error::Dns { .. }), "{err}");
}
