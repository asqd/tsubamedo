//! Collection runs against a local server: parallel requests, the order of those that take
//! values from each other, retries, and a failed login that is not sent again for each request.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tsubamedo_core::collection::Collection;
use tsubamedo_core::http::SendOptions;
use tsubamedo_core::run::{Event, RunOptions, Runner};
use tsubamedo_core::Vars;

#[derive(Default)]
struct Server {
    /// Requests per path.
    hits: Mutex<HashMap<String, usize>>,
    in_flight: AtomicUsize,
    most: AtomicUsize,
}

async fn handle(s: Arc<Server>, req: hyper::Request<Incoming>) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path().to_string();
    let n = {
        let mut hits = s.hits.lock().unwrap();
        let n = hits.entry(path.clone()).or_default();
        *n += 1;
        *n
    };
    let now = s.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
    s.most.fetch_max(now, Ordering::SeqCst);
    let authorized = req.headers().get("authorization").is_some_and(|v| v.as_bytes().starts_with(b"Bearer t-"));
    let (status, body) = match path.as_str() {
        "/login" => (200, r#"{"token": "t-1"}"#.to_string()),
        "/login-down" => (500, r#"{"error": "лежит"}"#.to_string()),
        "/slow" => {
            tokio::time::sleep(Duration::from_millis(300)).await;
            (200, "{}".into())
        }
        // Fails twice, then works.
        "/flaky" => (if n <= 2 { 503 } else { 200 }, "{}".into()),
        _ if authorized => (200, r#"{"ok": true}"#.into()),
        _ => (401, r#"{"error": "нужен токен"}"#.into()),
    };
    s.in_flight.fetch_sub(1, Ordering::SeqCst);
    Ok(hyper::Response::builder().status(status).header("content-type", "application/json").body(Full::new(Bytes::from(body))).unwrap())
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

/// A collection of `name → file text`, with `{{base}}` pointing at the server.
fn collection(addr: SocketAddr, files: &[(&str, &str)]) -> (tempfile::TempDir, Collection) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("tsubamedo.toml"), format!("name = \"Тест\"\n[vars]\nbase = \"http://{addr}\"\n")).unwrap();
    for (name, text) in files {
        let path = dir.path().join(format!("{name}.toml"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let c = Collection::open(dir.path()).unwrap();
    assert!(c.broken.is_empty(), "{:?}", c.broken);
    (dir, c)
}

/// The run's events as text: `+id` started (with the attempt from the second), `-id ✓/✗`.
async fn run(c: &Collection, options: RunOptions) -> Vec<String> {
    let mut runner = Runner::new(Some(c), None, Vars::new(), Vars::new(), SendOptions::default()).unwrap();
    let steps = runner.plan(c, |_| true);
    let mut log = Vec::new();
    runner
        .run(&steps, &options, |e| match e {
            Event::Started { step, attempt } => log.push(if attempt == 1 { format!("+{}", steps[step].id) } else { format!("+{} #{attempt}", steps[step].id) }),
            Event::Finished { step, passed, exchanges, .. } => {
                let extra: Vec<String> = exchanges.iter().rev().skip(1).filter_map(|x| x.id.clone()).collect();
                let extra = if extra.is_empty() { String::new() } else { format!(" (сначала {})", extra.join(", ")) };
                log.push(format!("-{} {}{extra}", steps[step].id, if passed { "✓" } else { "✗" }));
            }
        })
        .await;
    log
}

#[tokio::test(flavor = "multi_thread")]
async fn independent_requests_go_side_by_side() {
    let (addr, s) = server().await;
    let files: Vec<(String, String)> = (1..=4).map(|i| (format!("slow{i}"), "url = \"{{base}}/slow\"\n".to_string())).collect();
    let files: Vec<(&str, &str)> = files.iter().map(|(n, t)| (n.as_str(), t.as_str())).collect();
    let (_dir, c) = collection(addr, &files);

    let started = Instant::now();
    let log = run(&c, RunOptions { parallel: 4, ..RunOptions::default() }).await;
    assert!(started.elapsed() < Duration::from_millis(900), "{:?}", started.elapsed());
    assert_eq!(s.most.load(Ordering::SeqCst), 4);
    assert_eq!(log.iter().filter(|l| l.ends_with('✓')).count(), 4);

    // One at a time: the collection's order, never two at once.
    s.most.store(0, Ordering::SeqCst);
    let log = run(&c, RunOptions::default()).await;
    assert_eq!(log, ["+slow1", "-slow1 ✓", "+slow2", "-slow2 ✓", "+slow3", "-slow3 ✓", "+slow4", "-slow4 ✓"]);
    assert_eq!(s.most.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn those_that_need_a_value_wait_for_it() {
    let (addr, s) = server().await;
    let order = "url = \"{{base}}/orders\"\n[auth]\ntype = \"bearer\"\ntoken = \"{{token}}\"\n";
    let (_dir, c) = collection(
        addr,
        &[("a-orders", order), ("b-orders", order), ("c-orders", order), ("z-login", "method = \"POST\"\nurl = \"{{base}}/login\"\n[capture]\ntoken = '$.token'\n")],
    );
    let log = run(&c, RunOptions { parallel: 4, ..RunOptions::default() }).await;
    assert_eq!(&log[..2], ["+z-login", "-z-login ✓"], "{log:?}");
    assert_eq!(log.iter().filter(|l| l.ends_with('✓')).count(), 4, "{log:?}");
    assert_eq!(s.hits.lock().unwrap()["/login"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_requests_are_sent_again() {
    let (addr, s) = server().await;
    let (_dir, c) = collection(addr, &[("flaky", "url = \"{{base}}/flaky\"\nchecks = ['status == 200']\n")]);
    let log = run(&c, RunOptions { retries: 1, ..RunOptions::default() }).await;
    assert_eq!(log, ["+flaky", "+flaky #2", "-flaky ✗"]);
    let log = run(&c, RunOptions { retries: 3, ..RunOptions::default() }).await;
    assert_eq!(log, ["+flaky", "-flaky ✓"], "третий раз удачный");
    assert_eq!(s.hits.lock().unwrap()["/flaky"], 3);

    // Without checks a 503 is an answer; with fail_status it is a failure.
    let (_dir, c) = collection(addr, &[("down", "url = \"{{base}}/login-down\"\n")]);
    assert_eq!(run(&c, RunOptions::default()).await, ["+down", "-down ✓"]);
    let log = run(&c, RunOptions { fail_status: true, retries: 1, ..RunOptions::default() }).await;
    assert_eq!(log, ["+down", "+down #2", "-down ✗"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_login_is_not_sent_again_for_every_request() {
    let (addr, s) = server().await;
    let order = "url = \"{{base}}/orders\"\n[auth]\ntype = \"bearer\"\ntoken = \"{{token}}\"\n";
    let (_dir, c) = collection(
        addr,
        &[("login", "method = \"POST\"\nurl = \"{{base}}/login-down\"\n[capture]\ntoken = '$.token'\n"), ("orders1", order), ("orders2", order)],
    );
    let log = run(&c, RunOptions { parallel: 2, ..RunOptions::default() }).await;
    assert_eq!(&log[..2], ["+login", "-login ✗"], "{log:?}");
    let mut rest = log[2..].to_vec();
    rest.sort();
    assert_eq!(rest, ["+orders1", "+orders2", "-orders1 ✗", "-orders2 ✗"], "запросы, которым нужен токен, всё равно идут");
    assert_eq!(s.hits.lock().unwrap()["/login-down"], 1, "{log:?}");
    assert!(!log.iter().any(|l| l.contains("сначала")), "{log:?}");

    // With bail nothing starts after the failure.
    let log = run(&c, RunOptions { bail: true, ..RunOptions::default() }).await;
    assert_eq!(log, ["+login", "-login ✗"]);
}
