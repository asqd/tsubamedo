//! The window's commands end to end: a collection on disk, a local server, the viewer.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tsubamedo_session::Session;

/// Answers every request with the same JSON body.
async fn server(body: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = vec![0; 8192];
                let _ = stream.read(&mut buf).await;
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(body.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}

async fn call(s: &Arc<Session>, method: &str, args: Value) -> Value {
    s.clone().call(method, args).await.unwrap_or_else(|e| panic!("{method}: {e}"))
}

#[tokio::test]
async fn edit_send_and_view_a_big_json() {
    let rows: Vec<String> = (0..3000).map(|i| format!(r#"{{"id":{i},"city":"Сочи"}}"#)).collect();
    let base = server(format!("[{}]", rows.join(","))).await;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("tsubamedo.toml"), "name = \"Тест\"\n").unwrap();
    std::fs::create_dir(dir.path().join("env")).unwrap();
    std::fs::write(dir.path().join("env/local.toml"), format!("[vars]\nbase = \"{base}\"\n")).unwrap();
    let data = tempfile::tempdir().unwrap();
    let s = Session::with_data_dir(data.path().to_path_buf());
    let info = call(&s, "openCollection", json!({ "path": dir.path() })).await;
    assert_eq!(info["environments"], json!(["local"]));

    let created = call(&s, "createRequest", json!({ "folder": "tours", "name": "Список туров" })).await;
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(id, "tours/Список туров");
    let mut request = call(&s, "getRequest", json!({ "id": id })).await;
    request["url"] = json!("{{base}}/tours");
    request["headers"] = json!([{ "name": "Accept", "value": "application/json", "enabled": true }]);
    assert_eq!(call(&s, "saveRequest", json!({ "id": id, "request": request })).await["written"], json!(true));
    assert_eq!(call(&s, "saveRequest", json!({ "id": id, "request": request })).await["written"], json!(false));
    let file = std::fs::read_to_string(dir.path().join("tours/Список туров.toml")).unwrap();
    assert!(file.contains("url = \"{{base}}/tours\""), "{file}");

    let meta = call(&s, "send", json!({ "sendId": "1", "id": id, "request": request, "env": "local" })).await;
    assert_eq!((meta["status"].as_u64(), meta["isJson"].as_bool()), (Some(200), Some(true)));
    assert_eq!(meta["sent"]["url"], json!(format!("{base}/tours")));

    let view = call(&s, "view", json!({ "responseId": meta["responseId"], "pretty": true })).await;
    assert_eq!(view["lines"], json!(2 + 3000 * 4));
    let found = call(&s, "viewSearch", json!({ "viewId": view["viewId"], "query": "сочи" })).await;
    assert_eq!(found["total"], json!(3000));
    let lines = call(&s, "viewLines", json!({ "viewId": view["viewId"], "lines": [1, 3], "query": "сочи" })).await;
    assert_eq!(lines[0]["foldEnd"], json!(4));
    assert_eq!(lines[1]["text"], json!("    \"city\": \"Сочи\""));
    assert_eq!(lines[1]["marks"], json!([[13, 17]]));

    // Once no tab shows the response, its body and views go; newer responses stay.
    let response = meta["responseId"].as_u64().unwrap();
    let newer = call(&s, "send", json!({ "sendId": "3", "id": id, "request": request, "env": "local" })).await;
    call(&s, "retain", json!({ "responses": [], "seen": response })).await;
    let gone = s.clone().call("viewLines", json!({ "viewId": view["viewId"], "lines": [1] })).await.unwrap_err();
    assert_eq!(gone, "вид ответа уже закрыт");
    assert!(s.clone().call("view", json!({ "responseId": response, "pretty": false })).await.is_err());
    call(&s, "view", json!({ "responseId": newer["responseId"], "pretty": true })).await;

    let err = s.clone().call("send", json!({ "sendId": "2", "id": id, "request": request })).await.unwrap_err();
    assert_eq!(err, "не заданы переменные: base");

    let vars = call(&s, "vars", json!({ "env": "local" })).await;
    assert_eq!(vars, json!([{ "name": "base", "value": base, "source": "окружение local", "secret": false }]));
    let curl = call(&s, "curl", json!({ "id": id, "request": request, "env": "local" })).await;
    assert_eq!(curl, json!(format!("curl \\\n  '{base}/tours' \\\n  -H 'Accept: application/json' \\\n  -L")));
    let imported = call(&s, "importCurl", json!({ "text": curl })).await;
    assert_eq!(imported["url"], json!(format!("{base}/tours")));

    // History: the two sends that went out, with their bodies. The one stopped by a missing
    // variable never left, so it is not there.
    let history = call(&s, "history", json!({ "requestId": id })).await;
    assert_eq!(history.as_array().unwrap().len(), 2);
    assert_eq!((history[0]["status"].as_u64(), history[0]["hasBody"].as_bool()), (Some(200), Some(true)));
    let old = call(&s, "openHistory", json!({ "id": history[0]["id"] })).await;
    assert_eq!(old["request"]["url"], json!("{{base}}/tours"));
    let view = call(&s, "view", json!({ "responseId": old["response"]["responseId"], "pretty": true })).await;
    assert_eq!(view["lines"], json!(2 + 3000 * 4));

    // Renaming and moving keep the history attached to the request.
    let renamed = call(&s, "renameRequest", json!({ "id": id, "name": "Все туры" })).await;
    assert_eq!(renamed["id"], json!("tours/Все туры"));
    let moved = call(&s, "moveFolder", json!({ "folder": "tours", "parent": call(&s, "createFolder", json!({ "folder": "", "name": "Архив" })).await["id"] })).await;
    assert_eq!(moved["id"], json!("Архив/tours"));
    assert_eq!(moved["collection"]["folders"], json!(["Архив", "Архив/tours"]));
    assert_eq!(call(&s, "history", json!({ "requestId": "Архив/tours/Все туры" })).await.as_array().unwrap().len(), 2);
    call(&s, "clearHistory", json!({})).await;
    assert_eq!(call(&s, "history", json!({})).await, json!([]));
}

/// `/login` gives out a token; `/orders` answers only with it, and counts the logins.
async fn shop() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let logins = Arc::new(AtomicUsize::new(0));
    let counter = logins.clone();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let counter = counter.clone();
            tokio::spawn(async move {
                let mut buf = vec![0; 8192];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, body) = if path == "/login" {
                    let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    ("200 OK", format!(r#"{{"token":"t-{n}"}}"#))
                } else if head.to_ascii_lowercase().contains("authorization: bearer t-") {
                    ("200 OK", r#"{"items":[{"id":1},{"id":2}]}"#.to_string())
                } else {
                    ("401 Unauthorized", r#"{"error":"нужен токен"}"#.to_string())
                };
                let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(body.as_bytes()).await;
            });
        }
    });
    (format!("http://{addr}"), logins)
}

#[tokio::test]
async fn chains_checks_and_collection_runs() {
    use std::sync::atomic::Ordering;
    let (base, logins) = shop().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("tsubamedo.toml"), format!("name = \"Магазин\"\n[vars]\nbase = \"{base}\"\n")).unwrap();
    std::fs::create_dir_all(root.join("auth")).unwrap();
    std::fs::create_dir_all(root.join("orders")).unwrap();
    std::fs::write(root.join("auth/login.toml"), "method = \"POST\"\nurl = \"{{base}}/login\"\nchecks = ['status == 200']\n\n[capture]\ntoken = '$.token'\n").unwrap();
    let orders = "url = \"{{base}}/orders\"\nchecks = [\n  'status == 200',\n  '$.items count == 2',\n  '$.items[0].id == 1',\n]\n\n[auth]\ntype = \"bearer\"\ntoken = \"{{token}}\"\n";
    std::fs::write(root.join("orders/list.toml"), orders).unwrap();
    std::fs::write(root.join("orders/created.toml"), orders.replace("'status == 200'", "'status == 201'")).unwrap();
    let data = tempfile::tempdir().unwrap();
    let s = Session::with_data_dir(data.path().to_path_buf());
    call(&s, "openCollection", json!({ "path": root })).await;

    // Before anything is captured, the variable is promised by the request that captures it.
    let vars = call(&s, "vars", json!({ "env": null })).await;
    assert!(vars.as_array().unwrap().iter().any(|v| v["name"] == "token" && v["source"] == "из ответа «auth/login», придёт при отправке"), "{vars}");

    // `token` is not set: login goes first, then the request with its checks.
    let request = call(&s, "getRequest", json!({ "id": "orders/list" })).await;
    assert_eq!(request["checks"][1], json!({ "text": "$.items count == 2", "enabled": true }));
    let meta = call(&s, "send", json!({ "sendId": "1", "id": "orders/list", "request": request, "env": null })).await;
    assert_eq!(meta["status"], json!(200));
    assert_eq!(meta["before"], json!([{ "id": "auth/login", "status": 200, "passed": true, "failures": [] }]));
    assert!(meta["checks"].as_array().unwrap().iter().all(|c| c["ok"] == json!(true)), "{}", meta["checks"]);
    // Now it is, so the next send does not log in again.
    let meta = call(&s, "send", json!({ "sendId": "2", "id": "orders/list", "request": request, "env": null })).await;
    assert_eq!(meta["before"], json!([]));
    assert_eq!(logins.load(Ordering::SeqCst), 1);
    let vars = call(&s, "vars", json!({ "env": null })).await;
    assert!(vars.as_array().unwrap().iter().any(|v| v["name"] == "token" && v["value"] == "t-1" && v["source"] == "из ответа «auth/login»"), "{vars}");
    // Both went to history, login too.
    assert_eq!(call(&s, "history", json!({ "requestId": "auth/login" })).await.as_array().unwrap().len(), 1);

    // A failing check says what came.
    let created = call(&s, "getRequest", json!({ "id": "orders/created" })).await;
    let meta = call(&s, "send", json!({ "sendId": "3", "id": "orders/created", "request": created, "env": null })).await;
    assert_eq!(meta["checks"][0], json!({ "check": "status == 201", "ok": false, "actual": "200", "error": null }));

    // Captured values survive a restart.
    let again = Session::with_data_dir(data.path().to_path_buf());
    call(&again, "openCollection", json!({ "path": root })).await;
    let vars = call(&again, "vars", json!({ "env": null })).await;
    assert!(vars.as_array().unwrap().iter().any(|v| v["name"] == "token" && v["value"] == "t-1"), "{vars}");

    // A folder run brings in login although it is outside the folder… once the token is gone.
    call(&again, "clearHistory", json!({})).await;
    let run = call(&again, "runCollection", json!({ "sendId": "r", "folder": "orders", "env": null })).await;
    let ids: Vec<&str> = run["results"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["auth/login", "orders/created", "orders/list"]);
    let passed: Vec<bool> = run["results"].as_array().unwrap().iter().map(|r| r["passed"].as_bool().unwrap()).collect();
    assert_eq!(passed, [true, false, true]);
    assert_eq!(run["results"][1]["failures"], json!(["status == 201 — пришло 200"]));
    assert_eq!(logins.load(Ordering::SeqCst), 2);
    assert_eq!(run["steps"].as_array().unwrap().len(), 3);
    assert_eq!((run["results"][0]["step"].clone(), run["results"][0]["attempts"].clone()), (json!(0), json!(1)));

    // Again only what failed, side by side with retries, watched as it goes.
    let s2 = again.clone();
    let rerun = tokio::spawn(async move {
        call(&s2, "runCollection", json!({ "sendId": "r2", "only": ["orders/created"], "parallel": 4, "retries": 1, "env": null })).await
    });
    let mut seen_running = false;
    while !rerun.is_finished() {
        let progress = call(&again, "runProgress", json!({ "sendId": "r2" })).await;
        seen_running |= progress["steps"].as_array().is_some_and(|s| s.len() == 1);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let rerun = rerun.await.unwrap();
    assert!(seen_running, "ход запуска виден, пока он идёт");
    assert_eq!(rerun["results"].as_array().unwrap().len(), 1, "вход не повторяется: токен уже есть");
    assert_eq!((rerun["results"][0]["id"].clone(), rerun["results"][0]["attempts"].clone()), (json!("orders/created"), json!(2)));
    assert_eq!(call(&again, "runProgress", json!({ "sendId": "r2" })).await, Value::Null, "после конца прогресс убран");

    assert_eq!(call(&s, "validateCheck", json!({ "text": "$.id == {{id}}" })).await, Value::Null);
    assert!(call(&s, "validateCheck", json!({ "text": "status" })).await.is_string());
}

#[tokio::test]
async fn scripts_before_and_after() {
    let (base, logins) = shop().await;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("tsubamedo.toml"), format!("[vars]\nbase = \"{base}\"\n")).unwrap();
    // The token comes from a script, not from [capture]: it still makes login a producer.
    std::fs::write(
        root.join("login.toml"),
        "method = \"POST\"\nurl = \"{{base}}/login\"\n\n[scripts]\nafter = '''\nconst { token } = response.json();\nvars.set(\"token\", token);\nconsole.log(\"вошли\", token);\n'''\n",
    )
    .unwrap();
    std::fs::write(
        root.join("orders.toml"),
        "url = \"{{base}}/orders\"\nchecks = ['$.items count == {{expected}}']\n\n[headers]\nAuthorization = \"Bearer {{token}}\"\nX-Stamp = \"{{stamp}}\"\n\n[scripts]\nbefore = '''\nvars.set(\"stamp\", \"s-\" + vars.get(\"token\"));\nvars.set(\"expected\", 2);\n'''\nafter = '''\ntest(\"два заказа\", () => expect(response.json().items).toHaveLength(2));\ntest(\"штамп ушёл\", () => expect(request.headers.get(\"x-stamp\")).toBe(\"s-t-1\"));\n'''\n",
    )
    .unwrap();
    let data = tempfile::tempdir().unwrap();
    let s = Session::with_data_dir(data.path().to_path_buf());
    call(&s, "openCollection", json!({ "path": root })).await;
    let request = call(&s, "getRequest", json!({ "id": "orders" })).await;
    assert!(request["scripts"]["before"].as_str().unwrap().contains("vars.set(\"stamp\""));

    let meta = call(&s, "send", json!({ "sendId": "1", "id": "orders", "request": request, "env": null })).await;
    assert_eq!(meta["before"][0]["id"], json!("login"));
    let checks: Vec<(String, bool)> = meta["checks"].as_array().unwrap().iter().map(|c| (c["check"].as_str().unwrap().to_string(), c["ok"].as_bool().unwrap())).collect();
    assert_eq!(checks, [("два заказа".into(), true), ("штамп ушёл".into(), true), ("$.items count == {{expected}}".into(), true)], "{meta}");
    assert_eq!(logins.load(std::sync::atomic::Ordering::SeqCst), 1);
    let history = call(&s, "history", json!({ "requestId": "login" })).await;
    let login = call(&s, "openHistory", json!({ "id": history[0]["id"] })).await;
    assert_eq!(login["response"]["logs"], json!(["вошли t-1"]));

    // A script that throws before sending stops the request with its line.
    let mut broken = request.clone();
    broken["scripts"]["before"] = json!("vars.set('a', 1);\nnope();");
    let err = s.clone().call("send", json!({ "sendId": "2", "id": "orders", "request": broken, "env": null })).await.unwrap_err();
    assert!(err.starts_with("скрипт до запроса: ") && err.ends_with("(строка 2)"), "{err}");
}

/// Sets a session cookie on /login and wants it on /me; gives tokens on /token, wants one on /api.
async fn auth_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buf = vec![0; 8192];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
                let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, extra, body) = match path.as_str() {
                    "/login" => ("200 OK", "Set-Cookie: sid=42; Path=/; Max-Age=3600\r\n", "{}"),
                    "/me" if head.contains("cookie: sid=42") => ("200 OK", "", r#"{"user":"кот"}"#),
                    "/token" if head.contains("grant_type=client_credentials") => {
                        ("200 OK", "", r#"{"access_token":"eyJhbGciOiJIUzI1NiJ9.kot.signature","expires_in":600}"#)
                    }
                    "/api" if head.contains("authorization: bearer eyjhbgcioijiuzi1nij9.kot.signature") => ("200 OK", "", "{}"),
                    _ => ("401 Unauthorized", "", "{}"),
                };
                let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn cookies_and_tokens_last_between_starts() {
    let base = auth_server().await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("tsubamedo.toml"), format!("name = \"Вход\"\n[vars]\nbase = \"{base}\"\n")).unwrap();
    let data = tempfile::tempdir().unwrap();
    let s = Session::with_data_dir(data.path().to_path_buf());
    call(&s, "openCollection", json!({ "path": dir.path() })).await;
    let send = |url: &str, auth: Value| json!({ "sendId": "x", "request": { "name": null, "method": "GET", "url": url, "query": [], "headers": [], "auth": auth, "body": { "type": "none" }, "options": {} }, "env": null });

    call(&s, "send", send("{{base}}/login", json!({ "type": "none" }))).await;
    let cookies = call(&s, "cookies", json!({ "env": null })).await;
    assert_eq!((cookies[0]["name"].clone(), cookies[0]["domain"].clone()), (json!("sid"), json!("127.0.0.1")), "{cookies}");

    let oauth = json!({ "type": "oauth2", "grant": "client_credentials", "tokenUrl": "{{base}}/token", "clientId": "app" });
    let request = send("{{base}}/api", oauth.clone())["request"].clone();
    assert_eq!(call(&s, "oauthStatus", json!({ "request": request, "env": null })).await, Value::Null);
    let token = call(&s, "oauthToken", json!({ "request": request, "env": null })).await;
    assert_eq!(token["preview"], json!("eyJhbGci…ture"));

    // Another start of the app: both are still there.
    let s = Session::with_data_dir(data.path().to_path_buf());
    call(&s, "openCollection", json!({ "path": dir.path() })).await;
    let me = call(&s, "send", send("{{base}}/me", json!({ "type": "none" }))).await;
    assert_eq!(me["status"], json!(200));
    assert!(me["sent"]["headers"].as_array().unwrap().iter().any(|h| h == &json!(["Cookie", "sid=42"])), "{}", me["sent"]);
    let api = call(&s, "send", send("{{base}}/api", oauth)).await;
    assert_eq!((api["status"].clone(), api["logs"].clone()), (json!(200), json!([])), "the kept token, no new one");
    assert_eq!(call(&s, "oauthStatus", json!({ "request": request, "env": null })).await["preview"], json!("eyJhbGci…ture"));

    call(&s, "clearCookies", json!({ "env": null })).await;
    assert_eq!(call(&s, "send", send("{{base}}/me", json!({ "type": "none" }))).await["status"], json!(401));
    call(&s, "oauthForget", json!({ "request": request, "env": null })).await;
    assert_eq!(call(&s, "oauthStatus", json!({ "request": request, "env": null })).await, Value::Null);
}

#[tokio::test]
async fn proxy_settings_and_certificates() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("tsubamedo.toml"), "name = \"Банк\"\n\n[vars]\nbase = \"https://api.bank.test\"\n").unwrap();
    let data = tempfile::tempdir().unwrap();
    let s = Session::with_data_dir(data.path().to_path_buf());
    call(&s, "openCollection", json!({ "path": dir.path() })).await;

    assert_eq!(call(&s, "proxySettings", json!({})).await["proxy"], json!({ "mode": "system" }));
    let bad = s.clone().call("saveProxy", json!({ "proxy": { "mode": "custom", "url": "ftp://x" } })).await;
    assert!(bad.unwrap_err().contains("схема ftp"));
    call(&s, "saveProxy", json!({ "proxy": { "mode": "custom", "url": "proxy.corp:3128", "noProxy": ".corp" } })).await;
    let again = Session::with_data_dir(data.path().to_path_buf());
    assert_eq!(call(&again, "proxySettings", json!({})).await["proxy"], json!({ "mode": "custom", "url": "proxy.corp:3128", "noProxy": ".corp" }));

    let certs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tsubamedo-core/tests/certs");
    let list = json!([
        { "host": "api.bank.test", "cert": certs.join("client.pem"), "key": certs.join("client.key") },
        { "host": "*.partner.test", "cert": certs.join("client.p12"), "passphrase": "{{cert_password}}" },
    ]);
    let saved = call(&s, "saveCertificates", json!({ "env": null, "certificates": list })).await;
    assert_eq!(saved[0]["status"], json!({ "ok": true, "text": "client.pem: сертификатов в цепочке 1" }), "{saved}");
    assert_eq!(saved[1]["status"], json!({ "ok": false, "text": "не заданы переменные: cert_password" }));
    let manifest = std::fs::read_to_string(dir.path().join("tsubamedo.toml")).unwrap();
    assert!(manifest.starts_with("name = \"Банк\"\n\n[vars]\nbase = \"https://api.bank.test\"\n\n[[certificates]]\nhost = \"api.bank.test\"\n"), "{manifest}");
    assert!(!manifest.contains("key = \"\"") && manifest.contains("passphrase = \"{{cert_password}}\""), "{manifest}");
}

#[tokio::test]
async fn compares_environments_and_history() {
    let stage = server(r#"{"version": "1.2", "items": [{"id": 1, "price": 10}]}"#.to_string()).await;
    let prod = server(r#"{"version": "1.3", "items": [{"id": 1, "price": 12}, {"id": 2, "price": 5}]}"#.to_string()).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("tsubamedo.toml"), "name = \"Магазин\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("env")).unwrap();
    std::fs::write(dir.path().join("env/stage.toml"), format!("[vars]\nbase = \"{stage}\"\n")).unwrap();
    std::fs::write(dir.path().join("env/prod.toml"), format!("[vars]\nbase = \"{prod}\"\n")).unwrap();
    std::fs::write(dir.path().join("items.toml"), "url = \"{{base}}/items\"\n").unwrap();
    let data = tempfile::tempdir().unwrap();
    let s = Session::with_data_dir(data.path().to_path_buf());
    call(&s, "openCollection", json!({ "path": dir.path() })).await;
    let request = call(&s, "getRequest", json!({ "id": "items" })).await;
    let a = call(&s, "send", json!({ "sendId": "a", "id": "items", "request": request, "env": "stage" })).await;
    let b = call(&s, "send", json!({ "sendId": "b", "id": "items", "request": request, "env": "prod" })).await;
    let side = |m: &Value| json!({ "responseId": m["responseId"], "status": m["status"], "headers": m["headers"] });

    let diff = call(&s, "compare", json!({ "left": side(&a), "right": side(&b), "ignore": ["version"] })).await;
    assert_eq!(diff["status"], json!([200, 200]));
    assert_eq!(diff["headers"], json!([]), "Content-Length is one of the ever-changing");
    assert_eq!(diff["body"]["kind"], json!("json"));
    let paths: Vec<&str> = diff["body"]["changes"].as_array().unwrap().iter().map(|c| c["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["$.items[id=1].price", "$.items[id=2]"]);
    assert_eq!(diff["body"]["ignored"], json!(1));

    // With what came before: the stage answer from history, against itself now.
    let history = call(&s, "history", json!({ "requestId": "items" })).await;
    let stage_entry = history.as_array().unwrap().iter().find(|e| e["env"] == "stage").unwrap()["id"].clone();
    let same = call(&s, "compare", json!({ "left": side(&a), "right": { "historyId": stage_entry } })).await;
    assert_eq!(same["body"], json!({ "kind": "same", "size": 53 }));
}

#[tokio::test]
async fn code_in_other_languages() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("tsubamedo.toml"), "name = \"Код\"\n[vars]\nbase = \"https://api.test\"\n").unwrap();
    let s = Session::new();
    call(&s, "openCollection", json!({ "path": dir.path() })).await;
    let request = json!({ "name": null, "method": "POST", "url": "{{base}}/orders", "query": [], "headers": [{ "name": "Authorization", "value": "Bearer {{token}}", "enabled": true }],
        "auth": { "type": "none" }, "body": { "type": "multipart", "parts": [{ "name": "photo", "value": "нет-такого.png", "isFile": true, "enabled": true }] }, "options": {} });
    let code = call(&s, "code", json!({ "request": request, "env": null, "lang": "python-requests" })).await;
    let code = code.as_str().unwrap();
    // A file that is not there and a variable not set yet still give code.
    assert!(code.contains("\"Authorization\": \"Bearer <token>\"") && code.contains("open(\"нет-такого.png\", \"rb\")"), "{code}");
    let langs = call(&s, "languages", json!({})).await;
    assert_eq!(langs[0], json!({ "id": "curl", "group": "Shell", "name": "curl" }));
    assert!(langs.as_array().unwrap().len() >= 20);
}
