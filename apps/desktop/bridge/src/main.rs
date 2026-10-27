//! Dev only: the app's single `call` command over HTTP, so the interface runs in a plain browser
//! against the real core (`make bridge`, then http://localhost:1430/bridge.html). Only the
//! Vite dev origin may call it. Native dialogs are stand-ins in the page itself.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tsubamedo_session::Session;

const ADDR: &str = "127.0.0.1:1431";
const ORIGINS: [&str; 2] = ["http://localhost:1430", "http://127.0.0.1:1430"];

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind(ADDR).await.expect("port 1431 is busy");
    eprintln!("tsubamedo-bridge: http://{ADDR}, open http://localhost:1430/bridge.html");
    let session = Session::new();
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let session = session.clone();
        tokio::spawn(async move {
            if let Err(e) = serve(stream, session).await {
                eprintln!("tsubamedo-bridge: {e}");
            }
        });
    }
}

/// One request per connection: `POST /call` with `{method, args}`.
async fn serve(stream: TcpStream, session: Arc<Session>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    let mut length = 0;
    let mut origin = String::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).await?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "origin" => origin = value.trim().to_string(),
                _ => {}
            }
        }
    }
    let allowed = ORIGINS.contains(&origin.as_str());
    let (status, body) = if !allowed {
        ("403 Forbidden", json!({ "error": "only the Vite dev server may call the bridge" }))
    } else if line.starts_with("OPTIONS") {
        ("204 No Content", Value::Null)
    } else {
        let mut raw = vec![0; length];
        reader.read_exact(&mut raw).await?;
        let request: Value = serde_json::from_slice(&raw).unwrap_or(Value::Null);
        let method = request["method"].as_str().unwrap_or_default().to_string();
        match session.call(&method, request["args"].clone()).await {
            Ok(value) => ("200 OK", json!({ "ok": value })),
            Err(e) => ("200 OK", json!({ "error": e })),
        }
    };
    let body = if body.is_null() { String::new() } else { body.to_string() };
    let cors = if allowed {
        format!("Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Methods: POST\r\nAccess-Control-Allow-Headers: content-type\r\n")
    } else {
        String::new()
    };
    let head = format!(
        "HTTP/1.1 {status}\r\n{cors}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut stream = reader.into_inner();
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    stream.shutdown().await
}
