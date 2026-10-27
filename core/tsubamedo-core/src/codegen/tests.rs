use std::path::Path;

use super::*;
use crate::model::{Auth, Pair, Part, Request};
use crate::prepare::prepare_without_files;

fn spec(request: &Request) -> Spec {
    let vars: Vars = [("base".to_string(), "https://api.test".to_string())].into();
    let prepared = prepare_without_files(request, &vars, Path::new(".")).unwrap();
    Spec::new(&prepared, &request.body, &vars)
}

fn json_request() -> Request {
    Request {
        method: "POST".into(),
        url: "{{base}}/orders?dry=1".into(),
        headers: vec![Pair::new("X-Note", "it's \"кот\" $HOME")],
        body: Body::Json { content: "{\n  \"name\": \"Кот\",\n  \"tags\": [\"a\", null],\n  \"vip\": true\n}".into() },
        ..Request::default()
    }
}

/// Every body each target knows: none, JSON, text, form, multipart with a file, a file.
fn bodies() -> Vec<Request> {
    let base = Request { method: "PUT".into(), url: "{{base}}/x".into(), ..Request::default() };
    vec![
        Request { method: "GET".into(), ..base.clone() },
        json_request(),
        Request { body: Body::Text { content: "line 1\nline 2\t\u{1}".into(), content_type: None }, ..base.clone() },
        Request { body: Body::Form { fields: vec![Pair::new("a", "1 2"), Pair::new("b", "&")] }, ..base.clone() },
        Request {
            body: Body::Multipart {
                parts: vec![
                    Part { name: "title".into(), value: "Котик".into(), is_file: false, enabled: true },
                    Part { name: "photo".into(), value: "img/kot.png".into(), is_file: true, enabled: true },
                ],
            },
            ..base.clone()
        },
        Request { body: Body::File { path: "data.bin".into(), content_type: None }, ..base },
    ]
}

#[test]
fn every_target_writes_every_body() {
    for lang in LANGUAGES {
        for request in bodies() {
            let code = generate(lang.id, &spec(&request)).unwrap();
            assert!(code.contains("api.test"), "{}: {code}", lang.id);
            // curl leaves out the method its body implies.
            assert!(code.to_lowercase().contains(&request.method.to_lowercase()) || lang.id == "curl", "{}: {code}", lang.id);
            if let Body::Multipart { .. } = request.body {
                assert!(code.contains("kot.png") || code.contains("multipart/form-data здесь не собирается"), "{}: {code}", lang.id);
            }
        }
    }
    assert!(generate("cobol", &spec(&json_request())).unwrap_err().starts_with("неизвестный язык «cobol», есть: curl, httpie"));
}

#[test]
fn python_requests_takes_json_as_a_dict() {
    let code = generate("python-requests", &spec(&json_request())).unwrap();
    assert_eq!(
        code,
        r#"import requests

url = "https://api.test/orders?dry=1"
headers = {
    "X-Note": "it's \"кот\" $HOME",
}
payload = {
    "name": "Кот",
    "tags": [
        "a",
        None,
    ],
    "vip": True,
}

response = requests.request("POST", url, headers=headers, json=payload)
print(response.status_code)
print(response.text)
"#
    );
}

#[test]
fn fetch_and_go() {
    assert_eq!(
        generate("js-fetch", &spec(&json_request())).unwrap(),
        r#"const response = await fetch("https://api.test/orders?dry=1", {
  method: "POST",
  headers: {
    "X-Note": "it's \"кот\" $HOME",
    "Content-Type": "application/json",
  },
  body: JSON.stringify({
    "name": "Кот",
    "tags": [
      "a",
      null
    ],
    "vip": true
  }),
});
console.log(response.status);
console.log(await response.text());
"#
    );
    let go = generate("go", &spec(&json_request())).unwrap();
    assert!(go.contains(r#"payload := strings.NewReader("{\"name\":\"Кот\",\"tags\":[\"a\",null],\"vip\":true}")"#), "{go}");
    assert!(go.contains("\"strings\"") && !go.contains("\"os\""), "{go}");
}

#[test]
fn literals_escape_what_each_language_interpolates() {
    let s = spec(&json_request());
    assert!(generate("kotlin-okhttp", &s).unwrap().contains(r#""it's \"кот\" \$HOME""#));
    assert!(generate("dart-http", &s).unwrap().contains(r#"'it\'s "кот" \$HOME'"#));
    assert!(generate("php-curl", &s).unwrap().contains(r#"'X-Note: it\'s "кот" $HOME'"#));
    assert!(generate("curl", &s).unwrap().contains(r#"-H 'X-Note: it'\''s "кот" $HOME'"#));
    assert!(generate("powershell", &s).unwrap().contains(r#"'X-Note' = 'it''s "кот" $HOME'"#));
    let text = spec(&bodies()[2]);
    assert!(generate("swift", &text).unwrap().contains(r#""line 1\nline 2\t\u{1}""#));
    assert!(generate("c-libcurl", &text).unwrap().contains(r#""line 1\nline 2\t\001""#));
    assert!(generate("java-httpclient", &text).unwrap().contains(r#""line 1\nline 2\t\u0001""#));
}

#[test]
fn what_a_target_cannot_repeat_is_said() {
    let mut r = Request { url: "{{base}}/x".into(), auth: Auth::Digest { username: "u".into(), password: "p".into() }, ..Request::default() };
    r.options.verify_tls = Some(false);
    r.options.follow_redirects = Some(false);
    let curl = generate("curl", &spec(&r)).unwrap();
    assert!(curl.contains("--digest -u 'u:p'") && curl.contains("-k") && !curl.contains("-L"), "{curl}");
    let requests = generate("python-requests", &spec(&r)).unwrap();
    assert!(requests.contains("auth=HTTPDigestAuth(\"u\", \"p\"), verify=False, allow_redirects=False"), "{requests}");
    let swift = generate("swift", &spec(&r)).unwrap();
    assert!(swift.starts_with("// Digest-авторизация здесь не повторяется"), "{swift}");
    assert_eq!(swift.lines().take_while(|l| l.starts_with("//")).count(), 3);
    let php = generate("php-curl", &spec(&r)).unwrap();
    assert!(php.starts_with("<?php\n\n$curl"), "everything is supported, no notes: {php}");
}
