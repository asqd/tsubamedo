//! Generated code, run for real against a local echo server: curl, Python, Node, Go, Java,
//! Ruby, Swift and C, as many as this machine has. Ignored by default — it needs compilers:
//!
//! ```sh
//! cargo test -p tsubamedo-core --test codegen -- --ignored --nocapture
//! ```

use std::convert::Infallible;
use std::path::Path;
use std::process::Command;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tsubamedo_core::codegen::{Spec, generate};
use tsubamedo_core::prepare::prepare_without_files;
use tsubamedo_core::{Body, Pair, Part, Request, Vars};

/// Answers with what it got: method, target, Content-Type, X-Note and the body.
async fn echo(req: hyper::Request<Incoming>) -> Result<hyper::Response<Full<Bytes>>, Infallible> {
    let head = format!(
        "{} {}\nct: {}\nnote: {}\n",
        req.method(),
        req.uri(),
        req.headers().get("content-type").map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned()).unwrap_or_default(),
        req.headers().get("x-note").map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned()).unwrap_or_default(),
    );
    let body = req.into_body().collect().await.map(|b| b.to_bytes()).unwrap_or_default();
    let text = format!("{head}{}", String::from_utf8_lossy(&body));
    Ok(hyper::Response::new(Full::new(Bytes::from(text))))
}

async fn server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream), service_fn(echo)));
        }
    });
    format!("http://{addr}")
}

fn has(tool: &str) -> bool {
    Command::new("sh").arg("-c").arg(format!("command -v {tool}")).output().is_ok_and(|o| o.status.success())
}

/// Writes the code where it runs and runs it; `None` when the tool is missing here.
fn run(lang: &str, code: &str, dir: &Path) -> Option<String> {
    let write = |name: &str| {
        let path = dir.join(name);
        std::fs::write(&path, code).unwrap();
        path
    };
    let out = match lang {
        "curl" => Command::new("sh").arg(write("run.sh")).current_dir(dir).output(),
        "python-http-client" if has("python3") => Command::new("python3").arg(write("run.py")).current_dir(dir).output(),
        "node-fetch" | "node-http" if has("node") => Command::new("node").arg(write("run.mjs")).current_dir(dir).output(),
        "go" if has("go") => {
            write("main.go");
            Command::new("go").args(["run", "main.go"]).current_dir(dir).env("GO111MODULE", "off").output()
        }
        // macOS has a `java` that only asks to install Java.
        "java-httpclient" if Command::new("java").arg("-version").output().is_ok_and(|o| o.status.success()) => Command::new("java").arg(write("Main.java")).current_dir(dir).output(),
        "ruby" if has("ruby") => Command::new("ruby").arg(write("run.rb")).current_dir(dir).output(),
        "swift" if has("swift") => Command::new("swift").arg(write("main.swift")).current_dir(dir).output(),
        "c-libcurl" if has("cc") => {
            let src = write("run.c");
            let built = Command::new("cc").arg(&src).args(["-o", "run_c", "-lcurl"]).current_dir(dir).output().unwrap();
            assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
            Command::new(dir.join("run_c")).current_dir(dir).output()
        }
        _ => return None,
    }
    .unwrap();
    assert!(out.status.success(), "{lang}:\n{code}\n{}", String::from_utf8_lossy(&out.stderr));
    Some(String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs compilers and interpreters"]
async fn generated_code_runs() {
    let base = server().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("img")).unwrap();
    std::fs::write(dir.path().join("img/kot.png"), "PNGDATA").unwrap();
    std::fs::write(dir.path().join("data.bin"), "BINARY").unwrap();
    let note = Pair::new("X-Note", "it's \"cat\" $HOME \\n");
    let cases: Vec<(&str, Request, Vec<&str>)> = vec![
        (
            "json",
            Request {
                method: "POST".into(),
                url: format!("{base}/orders?dry=1"),
                headers: vec![note.clone()],
                body: Body::Json { content: "{\"name\": \"Кот\", \"vip\": true}".into() },
                ..Request::default()
            },
            vec!["POST /orders?dry=1", "ct: application/json", "note: it's \"cat\" $HOME \\n", "\"Кот\"", "true"],
        ),
        (
            "form",
            Request { method: "PUT".into(), url: format!("{base}/f"), body: Body::Form { fields: vec![Pair::new("a", "1 2"), Pair::new("b", "&")] }, ..Request::default() },
            vec!["PUT /f", "ct: application/x-www-form-urlencoded", "b=%26"],
        ),
        (
            "multipart",
            Request {
                method: "POST".into(),
                url: format!("{base}/upload"),
                body: Body::Multipart {
                    parts: vec![
                        Part { name: "title".into(), value: "Котик".into(), is_file: false, enabled: true },
                        Part { name: "photo".into(), value: "img/kot.png".into(), is_file: true, enabled: true },
                    ],
                },
                ..Request::default()
            },
            vec!["ct: multipart/form-data; boundary=", "Котик", "filename=\"kot.png\"", "PNGDATA"],
        ),
        (
            "file",
            Request { method: "POST".into(), url: format!("{base}/raw"), body: Body::File { path: "data.bin".into(), content_type: Some("application/octet-stream".into()) }, ..Request::default() },
            vec!["POST /raw", "BINARY"],
        ),
    ];
    let langs = ["curl", "python-http-client", "node-fetch", "node-http", "go", "java-httpclient", "ruby", "swift", "c-libcurl"];
    // Targets that leave multipart to a note.
    let no_multipart = ["python-http-client", "node-http", "java-httpclient", "swift"];
    let mut ran = Vec::new();
    for lang in langs {
        for (case, request, expect) in &cases {
            if *case == "multipart" && no_multipart.contains(&lang) {
                continue;
            }
            let prepared = prepare_without_files(request, &Vars::new(), dir.path()).unwrap();
            let code = generate(lang, &Spec::new(&prepared, &request.body, &Vars::new())).unwrap();
            let Some(out) = run(lang, &code, dir.path()) else { continue };
            for e in expect {
                assert!(out.contains(e), "{lang} {case}: no «{e}» in\n{out}\ncode:\n{code}");
            }
            ran.push(format!("{lang}/{case}"));
        }
    }
    eprintln!("ran: {}", ran.join(", "));
    assert!(ran.iter().any(|r| r.starts_with("curl/")));
}
