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
