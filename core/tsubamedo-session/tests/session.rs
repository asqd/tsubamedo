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
