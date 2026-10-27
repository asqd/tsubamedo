use std::path::Path;
use std::time::Duration;

use super::*;
use crate::http::{ResponseBody, Timings};
use crate::model::Auth;
use crate::prepare::prepare;

fn response(status: u16, body: &str) -> Response {
    Response {
        status,
        reason: "Created".into(),
        version: "HTTP/1.1".into(),
        headers: vec![("Content-Type".into(), "application/json".into()), ("X-Request-Id".into(), "r-1".into())],
        body: ResponseBody::Memory(bytes::Bytes::from(body.to_string())),
        wire_size: body.len() as u64,
        timings: Timings { total: Duration::from_millis(42), ..Timings::default() },
        url: "http://test/orders".into(),
        remote_addr: "127.0.0.1:80".parse().unwrap(),
        tls: None,
        redirects: Vec::new(),
        sent_cookie: None,
        via: None,
    }
}

fn vars(list: &[(&str, &str)]) -> Vars {
    list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn before_changes_the_request_and_sets_variables() {
    let request = Request {
        method: "post".into(),
        url: "{{base}}/orders".into(),
        headers: vec![Pair::new("Accept", "application/json")],
        auth: Auth::None,
        body: Body::Json { content: r#"{"tour": "{{tour}}"}"#.into() },
        ..Request::default()
    };
    let script = r#"
        vars.set("ts", 1700000000);
        vars.set("meta", { a: 1 });
        const body = vars.render(request.body);
        request.headers.set("X-Sign", crypto.hmac("sha256", vars.get("secret"), body));
        request.headers.set("accept", "*/*");
        request.query.set("v", 2);
        request.body = { tour: "{{tour}}", at: "{{ts}}" };
        console.log("тело", body, { n: 1 });
    "#;
    let v = vars(&[("base", "http://api.test"), ("tour", "Сочи"), ("secret", "key")]);
    let (changed, out) = before(script, &request, &v);
    assert_eq!(out.error, None);
    assert_eq!(out.vars, [("ts".to_string(), "1700000000".to_string()), ("meta".into(), r#"{"a":1}"#.into())]);
    assert_eq!(out.logs, [r#"тело {"tour": "Сочи"} {"n":1}"#]);
    // HMAC-SHA256("key", `{"tour": "Сочи"}`), checked with openssl.
    let sign = changed.headers.iter().find(|p| p.name == "X-Sign").unwrap();
    assert_eq!(sign.value, "836e53324440b60ceb298d1ca74c4f8bc2d2ab2c3402e9838d6de19b40134200");
    assert_eq!(changed.headers[0], Pair::new("Accept", "*/*"));
    assert_eq!(changed.query, [Pair::new("v", "2")]);
    assert_eq!(changed.body, Body::Json { content: "{\n  \"tour\": \"{{tour}}\",\n  \"at\": \"{{ts}}\"\n}".into() });

    // The variables it set reach this very request.
    let mut all = v.clone();
    all.extend(out.vars);
    let p = prepare(&changed, &all, Path::new(".")).unwrap();
    assert_eq!(std::str::from_utf8(&p.body).unwrap(), "{\n  \"tour\": \"Сочи\",\n  \"at\": \"1700000000\"\n}");
}

#[test]
fn after_reads_the_response_and_tests_it() {
    let request = Request { method: "POST".into(), url: "http://api.test/orders?x=1".into(), body: Body::Json { content: "{}".into() }, ..Request::default() };
    let sent = prepare(&request, &Vars::new(), Path::new(".")).unwrap();
    let r = response(201, r#"{"id": 42, "items": [1, 2], "status": "new"}"#);
    let script = r#"
        const order = response.json();
        vars.set("order_id", order.id);
        test("создан", () => expect(response.status).toBe(201));
        test("два товара", () => expect(order.items).toHaveLength(2));
        test("оплачен", () => expect(order.status).toBe("paid"));
        test("не пусто", () => expect(order.items).not.toEqual([]));
        test("быстро", () => expect(response.time).toBeLessThan(100));
        test("заголовок", () => expect(response.headers.get("x-request-id")).toMatch(/^r-/));
        test("запрос", () => expect(request.query.get("x")).toBe("1"));
        test("нельзя менять", () => request.headers.set("A", "b"));
        test("и тут", () => { request.url = "x" });
        test("поле", () => expect(order).toHaveProperty("items.length"));
        test("sha", () => expect(crypto.sha256("abc")).toBe("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        test("base64", () => expect(atob(btoa("Котик"))).toBe("Котик"));
    "#;
    let out = after(script, &sent, &r, &Vars::new());
    assert_eq!(out.error, None);
    assert_eq!(out.vars, [("order_id".to_string(), "42".to_string())]);
    let results: Vec<(&str, bool)> = out.tests.iter().map(|t| (t.name.as_str(), t.ok)).collect();
    assert_eq!(
        results,
        [
            ("создан", true),
            ("два товара", true),
            ("оплачен", false),
            ("не пусто", true),
            ("быстро", true),
            ("заголовок", true),
            ("запрос", true),
            ("нельзя менять", false),
            ("и тут", false),
            ("поле", true),
            ("sha", true),
            ("base64", true),
        ]
    );
    assert_eq!(out.tests[2].error.as_deref(), Some(r#"ожидалось "paid", пришло "new""#));
    assert!(out.tests[7].error.as_deref().unwrap().contains("уже отправлены"));
}

#[test]
fn errors_say_where_and_keep_what_was_done() {
    let r = response(200, "<html>");
    let sent = prepare(&Request { url: "http://x.test".into(), ..Request::default() }, &Vars::new(), Path::new(".")).unwrap();
    let out = after("console.log('до');\nvars.set('a', 1);\nresponse.json();", &sent, &r, &Vars::new());
    assert_eq!(out.logs, ["до"]);
    assert_eq!(out.vars, [("a".to_string(), "1".to_string())]);
    let error = out.error.unwrap();
    assert!(error.starts_with("ответ не JSON"), "{error}");
    assert!(error.ends_with("(строка 3)"), "{error}");

    let out = after("let x = ;", &sent, &r, &Vars::new());
    assert!(out.error.unwrap().contains("строка 1"));
    let out = after("throw 'строка'", &sent, &r, &Vars::new());
    assert!(out.error.is_some());
}

#[test]
fn a_runaway_script_is_stopped() {
    let started = Instant::now();
    let (_, out) = before("while (true) {}", &Request::default(), &Vars::new());
    assert!(out.error.unwrap().contains("дольше"));
    assert!(started.elapsed() < TIME_LIMIT + Duration::from_secs(2));
    let (_, out) = before("const a = []; while (true) a.push('x'.repeat(1 << 20));", &Request::default(), &Vars::new());
    assert!(out.error.is_some());
}

#[test]
fn no_files_no_network_no_modules() {
    for script in ["require('fs')", "fetch('http://x')", "std.open('/etc/passwd')", "os.exec(['ls'])", "new XMLHttpRequest()"] {
        let (_, out) = before(script, &Request::default(), &Vars::new());
        assert!(out.error.is_some(), "{script}");
    }
    // A dynamic import has nothing to load modules from: it only rejects its promise.
    let (_, out) = before("let got = 'нет'; import('os').then(() => got = 'да', (e) => got = String(e)).then(() => console.log(got))", &Request::default(), &Vars::new());
    assert_eq!(out.error, None);
    assert!(out.logs[0].contains("Error"), "{:?}", out.logs);
}

#[test]
fn postman_and_insomnia_scripts_run_as_they_are() {
    let request = Request { method: "POST".into(), url: "http://api.test/login".into(), body: Body::Json { content: "{}".into() }, ..Request::default() };
    let before = r#"
        const ts = String(1700000000);
        pm.environment.set("ts", ts);
        pm.request.headers.add({ key: "X-Sign", value: CryptoJS.HmacSHA256("{}", "key").toString(CryptoJS.enc.Base64) });
        pm.request.url.query.add({ key: "v", value: "2" });
        console.log(pm.variables.replaceIn("{{base}}/x"), pm.info.eventName);
    "#;
    let (changed, out) = super::before(before, &request, &vars(&[("base", "http://api.test")]));
    assert_eq!(out.error, None);
    assert_eq!(out.vars, [("ts".to_string(), "1700000000".to_string())]);
    assert_eq!(out.logs, ["http://api.test/x prerequest"]);
    // HMAC-SHA256("key", "{}") in base64, checked with openssl.
    let sign = changed.headers.iter().find(|p| p.name == "X-Sign").unwrap();
    assert_eq!(sign.value, "p3dyTZQ+tI3Gm8qKSm1XoE2z+ex+HeTlgehgJlvfMDI=");
    assert_eq!(changed.query, [Pair::new("v", "2")]);

    let sent = prepare(&changed, &Vars::new(), Path::new(".")).unwrap();
    let r = response(201, r#"{"token": "t-1", "user": {"roles": ["admin"]}, "items": []}"#);
    let after = r#"
        pm.test("создан", () => pm.response.to.have.status(201));
        pm.test("не ошибка", () => pm.response.to.not.be.error);
        pm.test("ok — это 200", () => pm.response.to.be.ok);
        const body = pm.response.json();
        pm.test("токен", () => {
            pm.expect(body.token).to.be.a("string").and.not.empty;
            pm.expect(body).to.have.property("user").that.has.property("roles");
            pm.expect(body.user.roles).to.include("admin");
            pm.expect(body.items).to.be.an("array").that.is.empty;
            pm.expect(pm.response.responseTime).to.be.below(1000);
        });
        pm.test("deep", () => pm.expect(body.user).to.eql({ roles: ["admin"] }));
        pm.test("заголовок", () => pm.response.to.have.header("X-Request-Id", "r-1"));
        insomnia.test("insomnia", () => insomnia.expect(insomnia.response.code).to.equal(201));
        tests["старый стиль"] = responseCode.code === 201;
        tests["ложь"] = false;
        pm.collectionVariables.set("token", body.token);
        postman.setEnvironmentVariable("user", "u-1");
    "#;
    let out = super::after(after, &sent, &r, &Vars::new());
    assert_eq!(out.error, None);
    let results: Vec<(&str, bool)> = out.tests.iter().map(|t| (t.name.as_str(), t.ok)).collect();
    assert_eq!(
        results,
        [
            ("создан", true),
            ("не ошибка", true),
            ("ok — это 200", false),
            ("токен", true),
            ("deep", true),
            ("заголовок", true),
            ("insomnia", true),
            ("старый стиль", true),
            ("ложь", false)
        ]
    );
    assert_eq!(out.vars, [("token".to_string(), "t-1".to_string()), ("user".into(), "u-1".into())]);
    assert_eq!(sets(after), ["token", "user"]);
    assert_eq!(reads("pm.environment.get('a'); insomnia.environment.get(\"b\"); postman.getGlobalVariable('c')"), ["a", "b", "c"]);
}
