//! One function per target: method, address, headers, body, then what the library can repeat
//! of the rest. Each prints the status and the body, so the code runs as it is.

use std::fmt::Write;

use super::{Ctrl, Field, Payload, Spec, dq, file_name, js, js_value, python_value, sh, sq};

pub(super) fn write(id: &str, s: &Spec) -> String {
    match id {
        "curl" => curl(s),
        "httpie" => httpie(s),
        "wget" => wget(s),
        "powershell" => powershell(s),
        "python-requests" => python_requests(s),
        "python-http-client" => python_http_client(s),
        "js-fetch" => fetch(s, false),
        "node-fetch" => fetch(s, true),
        "js-axios" => axios(s, false),
        "node-axios" => axios(s, true),
        "js-xhr" => xhr(s),
        "node-http" => node_http(s),
        "go" => go(s),
        "java-httpclient" => java_httpclient(s),
        "java-okhttp" => okhttp(s, false),
        "kotlin-okhttp" => okhttp(s, true),
        "csharp-httpclient" => csharp(s),
        "php-curl" => php_curl(s),
        "php-guzzle" => php_guzzle(s),
        "ruby" => ruby(s),
        "swift" => swift(s),
        "rust-reqwest" => rust(s),
        "dart-http" => dart(s),
        "c-libcurl" => c_libcurl(s),
        "http" => raw_http(s),
        _ => unreachable!("languages are checked before"),
    }
}

/// Lines joined with ` \` for the shell.
fn shell(parts: Vec<String>) -> String {
    parts.join(" \\\n  ")
}

fn curl(s: &Spec) -> String {
    let mut p = vec!["curl".to_string()];
    let implied = if matches!(s.body, Payload::None) { "GET" } else { "POST" };
    if s.method != implied {
        p.push(format!("-X {}", s.method));
    }
    p.push(sh(s.url.as_str()));
    p.extend(s.headers.iter().map(|(n, v)| format!("-H {}", sh(&format!("{n}: {v}")))));
    match &s.body {
        Payload::None => {}
        Payload::Text(t) | Payload::Json(_, t) => p.push(format!("--data-raw {}", sh(t))),
        Payload::Form(_) => p.push(format!("--data-raw {}", sh(&s.text().unwrap_or_default()))),
        Payload::Multipart(fields) => {
            for f in fields {
                if f.file {
                    p.push(format!("-F {}", sh(&format!("{}=@{}", f.name, f.value))));
                } else {
                    p.push(format!("--form-string {}", sh(&format!("{}={}", f.name, f.value))));
                }
            }
        }
        Payload::File(path) => p.push(format!("--data-binary {}", sh(&format!("@{path}")))),
    }
    if let Some((u, pw)) = &s.digest {
        p.push(format!("--digest -u {}", sh(&format!("{u}:{pw}"))));
    }
    if s.insecure {
        p.push("-k".into());
    }
    if s.follow {
        p.push("-L".into());
    }
    shell(p)
}

fn httpie(s: &Spec) -> String {
    let mut p = vec!["http".to_string()];
    let file = matches!(s.body, Payload::File(_));
    if !file {
        p[0].push_str(" --ignore-stdin");
    }
    match &s.body {
        Payload::Form(_) => p.push("--form".into()),
        Payload::Multipart(_) => p.push("--multipart".into()),
        _ => {}
    }
    if let Some((u, pw)) = &s.digest {
        p.push(format!("-A digest -a {}", sh(&format!("{u}:{pw}"))));
    }
    if s.insecure {
        p.push("--verify=no".into());
    }
    if s.follow {
        p.push("--follow".into());
    }
    p.push(format!("{} {}", s.method, sh(s.url.as_str())));
    let form = matches!(s.body, Payload::Form(_) | Payload::Multipart(_));
    p.extend(s.headers.iter().filter(|(n, _)| !(form && n.eq_ignore_ascii_case("content-type"))).map(|(n, v)| sh(&format!("{n}:{v}"))));
    match &s.body {
        Payload::Text(t) | Payload::Json(_, t) => p.push(format!("--raw {}", sh(t))),
        Payload::Form(fields) => p.extend(fields.iter().map(|(k, v)| sh(&format!("{k}={v}")))),
        Payload::Multipart(fields) => p.extend(fields.iter().map(|f| sh(&format!("{}{}{}", f.name, if f.file { "@" } else { "=" }, f.value)))),
        Payload::File(path) => p.push(format!("< {}", sh(path))),
        Payload::None => {}
    }
    shell(p)
}

fn wget(s: &Spec) -> String {
    let mut p = vec!["wget --quiet".to_string(), format!("--method={}", s.method)];
    p.extend(s.headers.iter().map(|(n, v)| format!("--header={}", sh(&format!("{n}: {v}")))));
    match &s.body {
        Payload::File(path) => p.push(format!("--body-file={}", sh(path))),
        _ => {
            if let Some(t) = s.text() {
                p.push(format!("--body-data={}", sh(&t)));
            }
        }
    }
    if let Some((u, pw)) = &s.digest {
        p.push(format!("--user={} --password={}", sh(u), sh(pw)));
    }
    if s.insecure {
        p.push("--no-check-certificate".into());
    }
    if !s.follow {
        p.push("--max-redirect=0".into());
    }
    p.push("--output-document=-".into());
    p.push(sh(s.url.as_str()));
    shell(p)
}

/// PowerShell: single quotes, a quote doubled.
fn ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn powershell(s: &Spec) -> String {
    let mut o = String::new();
    let headers: Vec<_> = s.other_headers().collect();
    if !headers.is_empty() {
        o.push_str("$headers = @{\n");
        for (n, v) in &headers {
            let _ = writeln!(o, "    {} = {}", ps(n), ps(v));
        }
        o.push_str("}\n");
    }
    let mut args = vec![format!("-Uri {}", ps(s.url.as_str())), format!("-Method {}", s.method)];
    if !headers.is_empty() {
        args.push("-Headers $headers".into());
    }
    match &s.body {
        Payload::None => {}
        Payload::File(path) => args.push(format!("-InFile {}", ps(path))),
        Payload::Multipart(fields) => {
            o.push_str("$form = @{\n");
            for f in fields {
                let value = if f.file { format!("Get-Item {}", ps(&f.value)) } else { ps(&f.value) };
                let _ = writeln!(o, "    {} = {value}", ps(&f.name));
            }
            o.push_str("}\n");
            args.push("-Form $form".into());
        }
        _ => {
            let _ = writeln!(o, "$body = {}", ps(&s.text().unwrap_or_default()));
            args.push("-Body $body".into());
        }
    }
    if let Some(ct) = s.content_type() {
        args.push(format!("-ContentType {}", ps(ct)));
    }
    if s.insecure {
        args.push("-SkipCertificateCheck".into());
    }
    if !s.follow {
        args.push("-MaximumRedirection 0".into());
    }
    let _ = writeln!(o, "$response = Invoke-WebRequest {}", args.join(" "));
    o.push_str("$response.StatusCode\n$response.Content\n");
    o
}

fn python_requests(s: &Spec) -> String {
    let mut o = String::from("import requests\n");
    if s.digest.is_some() {
        o.push_str("from requests.auth import HTTPDigestAuth\n");
    }
    let _ = writeln!(o, "\nurl = {}", js(s.url.as_str()));
    let mut args = vec![js(&s.method), "url".to_string()];
    // `json=` writes Content-Type itself.
    let json = matches!(s.body, Payload::Json(..)) && s.content_type().is_none_or(|c| c.starts_with("application/json"));
    let headers: Vec<&(String, String)> = s.headers.iter().filter(|(n, _)| !(json && n.eq_ignore_ascii_case("content-type"))).collect();
    if !headers.is_empty() {
        o.push_str("headers = {\n");
        for (n, v) in &headers {
            let _ = writeln!(o, "    {}: {},", js(n), js(v));
        }
        o.push_str("}\n");
        args.push("headers=headers".into());
    }
    match &s.body {
        Payload::None => {}
        Payload::Json(v, _) if json => {
            let _ = writeln!(o, "payload = {}", python_value(v, 0));
            args.push("json=payload".into());
        }
        Payload::Text(_) | Payload::Json(..) => {
            let _ = writeln!(o, "payload = {}", js(&s.text().unwrap_or_default()));
            args.push("data=payload.encode(\"utf-8\")".into());
        }
        Payload::Form(fields) => {
            o.push_str("payload = [\n");
            for (k, v) in fields {
                let _ = writeln!(o, "    ({}, {}),", js(k), js(v));
            }
            o.push_str("]\n");
            args.push("data=payload".into());
        }
        Payload::Multipart(fields) => {
            let (files, texts): (Vec<&Field>, Vec<&Field>) = fields.iter().partition(|f| f.file);
            if !texts.is_empty() {
                o.push_str("data = [\n");
                for f in &texts {
                    let _ = writeln!(o, "    ({}, {}),", js(&f.name), js(&f.value));
                }
                o.push_str("]\n");
                args.push("data=data".into());
            }
            o.push_str("files = [\n");
            for f in &files {
                let _ = writeln!(o, "    ({}, ({}, open({}, \"rb\"))),", js(&f.name), js(&file_name(&f.value)), js(&f.value));
            }
            o.push_str("]\n");
            args.push("files=files".into());
        }
        Payload::File(path) => {
            let _ = writeln!(o, "payload = open({}, \"rb\")", js(path));
            args.push("data=payload".into());
        }
    }
    if let Some((u, p)) = &s.digest {
        args.push(format!("auth=HTTPDigestAuth({}, {})", js(u), js(p)));
    }
    if s.insecure {
        args.push("verify=False".into());
    }
    if !s.follow {
        args.push("allow_redirects=False".into());
    }
    let _ = writeln!(o, "\nresponse = requests.request({})", args.join(", "));
    o.push_str("print(response.status_code)\nprint(response.text)\n");
    o
}

fn python_http_client(s: &Spec) -> String {
    let mut o = String::from("import http.client\n");
    if s.insecure {
        o.push_str("import ssl\n");
    }
    let host = s.url.host_str().unwrap_or_default();
    let host = match s.url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    };
    let class = if s.is_https() { "HTTPSConnection" } else { "HTTPConnection" };
    let context = if s.insecure && s.is_https() { ", context=ssl._create_unverified_context()" } else { "" };
    let _ = writeln!(o, "\nconn = http.client.{class}({}{context})", js(&host));
    let body = match &s.body {
        Payload::File(path) => {
            let _ = writeln!(o, "payload = open({}, \"rb\").read()", js(path));
            "payload"
        }
        _ => match s.text() {
            Some(t) => {
                let _ = writeln!(o, "payload = {}.encode(\"utf-8\")", js(&t));
                "payload"
            }
            None => "None",
        },
    };
    o.push_str("headers = {\n");
    for (n, v) in &s.headers {
        let _ = writeln!(o, "    {}: {},", js(n), js(v));
    }
    o.push_str("}\n");
    let _ = writeln!(o, "conn.request({}, {}, {body}, headers)", js(&s.method), js(s.target()));
    o.push_str("response = conn.getresponse()\nprint(response.status)\nprint(response.read().decode(\"utf-8\"))\n");
    o
}

/// The JavaScript body expression, with what has to come before it.
fn js_body(s: &Spec, node: bool) -> (String, Option<String>) {
    match &s.body {
        Payload::None => (String::new(), None),
        Payload::Json(v, _) => (String::new(), Some(format!("JSON.stringify({})", js_value(v, 2)))),
        Payload::Text(t) => (String::new(), Some(js(t))),
        Payload::Form(fields) => {
            let pairs: Vec<String> = fields.iter().map(|(k, v)| format!("[{}, {}]", js(k), js(v))).collect();
            (String::new(), Some(format!("new URLSearchParams([{}])", pairs.join(", "))))
        }
        Payload::Multipart(fields) => {
            let mut pre = String::from("const form = new FormData();\n");
            for f in fields {
                if !f.file {
                    let _ = writeln!(pre, "form.append({}, {});", js(&f.name), js(&f.value));
                } else if node {
                    let _ = writeln!(pre, "form.append({}, await openAsBlob({}), {});", js(&f.name), js(&f.value), js(&file_name(&f.value)));
                } else {
                    let _ = writeln!(pre, "form.append({}, fileInput.files[0], {}); // {}", js(&f.name), js(&file_name(&f.value)), f.value);
                }
            }
            (pre, Some("form".into()))
        }
        Payload::File(path) if node => (String::new(), Some(format!("await openAsBlob({})", js(path)))),
        Payload::File(path) => (String::new(), Some(format!("file /* {path}: Blob или File из <input type=\"file\"> */"))),
    }
}

/// Headers as a JavaScript object; multipart and forms leave Content-Type to the library.
fn js_headers(s: &Spec, indent: usize) -> Option<String> {
    let skip_type = matches!(s.body, Payload::Form(_));
    let list: Vec<_> = s.headers.iter().filter(|(n, _)| !(skip_type && n.eq_ignore_ascii_case("content-type"))).collect();
    if list.is_empty() {
        return None;
    }
    let pad = " ".repeat(indent + 2);
    let inner: String = list.iter().map(|(n, v)| format!("{pad}{}: {},\n", js(n), js(v))).collect();
    Some(format!("{{\n{inner}{}}}", " ".repeat(indent)))
}

fn node_imports(s: &Spec) -> String {
    let files = matches!(s.body, Payload::File(_)) || matches!(&s.body, Payload::Multipart(f) if f.iter().any(|f| f.file));
    if files { "import { openAsBlob } from \"node:fs\";\n\n".into() } else { String::new() }
}

fn fetch(s: &Spec, node: bool) -> String {
    let mut o = if node { node_imports(s) } else { String::new() };
    let (pre, body) = js_body(s, node);
    o.push_str(&pre);
    let mut opts = vec![format!("  method: {},", js(&s.method))];
    if let Some(h) = js_headers(s, 2) {
        opts.push(format!("  headers: {h},"));
    }
    if let Some(b) = body {
        opts.push(format!("  body: {b},"));
    }
    if !s.follow {
        opts.push("  redirect: \"manual\",".into());
    }
    let _ = writeln!(o, "const response = await fetch({}, {{\n{}\n}});", js(s.url.as_str()), opts.join("\n"));
    o.push_str("console.log(response.status);\nconsole.log(await response.text());\n");
    o
}

fn axios(s: &Spec, node: bool) -> String {
    let mut o = String::from("import axios from \"axios\";\n");
    if node && s.insecure && s.is_https() {
        o.push_str("import https from \"node:https\";\n");
    }
    if node {
        o.push_str(&node_imports(s));
    }
    if !o.ends_with("\n\n") {
        o.push('\n');
    }
    let (pre, body) = js_body(s, node);
    o.push_str(&pre);
    let mut opts = vec![format!("  method: {},", js(&s.method)), format!("  url: {},", js(s.url.as_str()))];
    if let Some(h) = js_headers(s, 2) {
        opts.push(format!("  headers: {h},"));
    }
    if let Some(b) = body {
        // axios turns objects into JSON itself.
        let b = match &s.body {
            Payload::Json(v, _) => js_value(v, 2),
            _ => b,
        };
        opts.push(format!("  data: {b},"));
    }
    if node && !s.follow {
        opts.push("  maxRedirects: 0,".into());
    }
    if node && s.insecure && s.is_https() {
        opts.push("  httpsAgent: new https.Agent({ rejectUnauthorized: false }),".into());
    }
    opts.push("  validateStatus: () => true,".into());
    let _ = writeln!(o, "const response = await axios.request({{\n{}\n}});", opts.join("\n"));
    o.push_str("console.log(response.status);\nconsole.log(response.data);\n");
    o
}

fn xhr(s: &Spec) -> String {
    let (pre, body) = js_body(s, false);
    let mut o = pre;
    o.push_str("const xhr = new XMLHttpRequest();\n");
    let _ = writeln!(o, "xhr.open({}, {});", js(&s.method), js(s.url.as_str()));
    let skip_type = matches!(s.body, Payload::Form(_));
    for (n, v) in s.headers.iter().filter(|(n, _)| !(skip_type && n.eq_ignore_ascii_case("content-type"))) {
        let _ = writeln!(o, "xhr.setRequestHeader({}, {});", js(n), js(v));
    }
    o.push_str("xhr.addEventListener(\"load\", () => {\n  console.log(xhr.status);\n  console.log(xhr.responseText);\n});\n");
    let _ = writeln!(o, "xhr.send({});", body.unwrap_or_default());
    o
}

fn node_http(s: &Spec) -> String {
    let module = if s.is_https() { "https" } else { "http" };
    let mut o = format!("import {module} from \"node:{module}\";\n");
    if matches!(s.body, Payload::File(_)) {
        o.push_str("import fs from \"node:fs\";\n");
    }
    o.push('\n');
    let mut opts = vec![format!("  method: {},", js(&s.method))];
    if !s.headers.is_empty() {
        let inner: String = s.headers.iter().map(|(n, v)| format!("    {}: {},\n", js(n), js(v))).collect();
        opts.push(format!("  headers: {{\n{inner}  }},"));
    }
    if s.insecure && s.is_https() {
        opts.push("  rejectUnauthorized: false,".into());
    }
    let _ = writeln!(o, "const request = {module}.request({}, {{\n{}\n}}, (response) => {{", js(s.url.as_str()), opts.join("\n"));
    o.push_str("  const chunks = [];\n  response.on(\"data\", (chunk) => chunks.push(chunk));\n  response.on(\"end\", () => {\n    console.log(response.statusCode);\n    console.log(Buffer.concat(chunks).toString());\n  });\n});\n");
    match &s.body {
        Payload::File(path) => {
            let _ = writeln!(o, "fs.createReadStream({}).pipe(request);", js(path));
        }
        _ => {
            if let Some(t) = s.text() {
                let _ = writeln!(o, "request.write({});", js(&t));
            }
            o.push_str("request.end();\n");
        }
    }
    o
}

fn go(s: &Spec) -> String {
    let q = |x: &str| dq(x, Ctrl::U4, &[]);
    let mut imports = vec!["fmt", "io", "net/http"];
    let mut body = String::new();
    let mut reader = "nil".to_string();
    let mut content_type = None;
    match &s.body {
        Payload::None => {}
        Payload::Multipart(fields) => {
            imports.extend(["bytes", "mime/multipart"]);
            if fields.iter().any(|f| f.file) {
                imports.push("os");
            }
            body.push_str("\tpayload := &bytes.Buffer{}\n\twriter := multipart.NewWriter(payload)\n");
            for f in fields {
                if f.file {
                    // A block each, so several files do not clash.
                    let _ = write!(
                        body,
                        "\t{{\n\t\tfile, err := os.Open({})\n\t\tif err != nil {{\n\t\t\tpanic(err)\n\t\t}}\n\t\tpart, _ := writer.CreateFormFile({}, {})\n\t\tio.Copy(part, file)\n\t\tfile.Close()\n\t}}\n",
                        q(&f.value),
                        q(&f.name),
                        q(&file_name(&f.value))
                    );
                } else {
                    let _ = writeln!(body, "\twriter.WriteField({}, {})", q(&f.name), q(&f.value));
                }
            }
            body.push_str("\twriter.Close()\n");
            reader = "payload".into();
            content_type = Some("writer.FormDataContentType()".to_string());
        }
        Payload::File(path) => {
            imports.push("os");
            let _ = write!(body, "\tpayload, err := os.Open({})\n\tif err != nil {{\n\t\tpanic(err)\n\t}}\n\tdefer payload.Close()\n", q(path));
            reader = "payload".into();
        }
        _ => {
            imports.push("strings");
            let _ = writeln!(body, "\tpayload := strings.NewReader({})", q(&s.text().unwrap_or_default()));
            reader = "payload".into();
        }
    }
    if s.insecure {
        imports.push("crypto/tls");
    }
    imports.sort();
    imports.dedup();
    let mut o = String::from("package main\n\nimport (\n");
    for i in &imports {
        let _ = writeln!(o, "\t\"{i}\"");
    }
    o.push_str(")\n\nfunc main() {\n");
    o.push_str(&body);
    let _ = writeln!(o, "\treq, err := http.NewRequest({}, {}, {reader})", q(&s.method), q(s.url.as_str()));
    o.push_str("\tif err != nil {\n\t\tpanic(err)\n\t}\n");
    for (n, v) in &s.headers {
        let _ = writeln!(o, "\treq.Header.Add({}, {})", q(n), q(v));
    }
    if let Some(ct) = content_type {
        let _ = writeln!(o, "\treq.Header.Set(\"Content-Type\", {ct})");
    }
    o.push_str("\tclient := &http.Client{");
    let mut fields = Vec::new();
    if s.insecure {
        fields.push("\n\t\tTransport: &http.Transport{TLSClientConfig: &tls.Config{InsecureSkipVerify: true}},".to_string());
    }
    if !s.follow {
        fields.push("\n\t\tCheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse },".to_string());
    }
    if !fields.is_empty() {
        o.push_str(&fields.concat());
        o.push_str("\n\t");
    }
    o.push_str("}\n\tres, err := client.Do(req)\n\tif err != nil {\n\t\tpanic(err)\n\t}\n\tdefer res.Body.Close()\n\tbody, _ := io.ReadAll(res.Body)\n\tfmt.Println(res.StatusCode)\n\tfmt.Println(string(body))\n}\n");
    o
}

/// Headers Java's HttpClient refuses to set.
const JAVA_RESTRICTED: &[&str] = &["host", "connection", "content-length", "expect", "upgrade"];

fn java_httpclient(s: &Spec) -> String {
    let q = |x: &str| dq(x, Ctrl::U4, &[]);
    let mut o = String::from("import java.net.URI;\nimport java.net.http.HttpClient;\nimport java.net.http.HttpRequest;\nimport java.net.http.HttpResponse;\n");
    if matches!(s.body, Payload::File(_)) {
        o.push_str("import java.nio.file.Path;\n");
    }
    o.push_str("\npublic class Main {\n    public static void main(String[] args) throws Exception {\n");
    let redirect = if s.follow { "NORMAL" } else { "NEVER" };
    let _ = writeln!(o, "        HttpClient client = HttpClient.newBuilder().followRedirects(HttpClient.Redirect.{redirect}).build();");
    let publisher = match &s.body {
        Payload::File(path) => format!("HttpRequest.BodyPublishers.ofFile(Path.of({}))", q(path)),
        Payload::None | Payload::Multipart(_) => "HttpRequest.BodyPublishers.noBody()".into(),
        _ => format!("HttpRequest.BodyPublishers.ofString({})", q(&s.text().unwrap_or_default())),
    };
    let _ = writeln!(o, "        HttpRequest request = HttpRequest.newBuilder()\n            .uri(URI.create({}))", q(s.url.as_str()));
    for (n, v) in s.headers.iter().filter(|(n, _)| !JAVA_RESTRICTED.contains(&n.to_ascii_lowercase().as_str())) {
        let _ = writeln!(o, "            .header({}, {})", q(n), q(v));
    }
    let _ = writeln!(o, "            .method({}, {publisher})\n            .build();", q(&s.method));
    o.push_str("        HttpResponse<String> response = client.send(request, HttpResponse.BodyHandlers.ofString());\n");
    o.push_str("        System.out.println(response.statusCode());\n        System.out.println(response.body());\n    }\n}\n");
    o
}

/// Java and Kotlin with OkHttp 4.
fn okhttp(s: &Spec, kotlin: bool) -> String {
    let q = |x: &str| if kotlin { dq(x, Ctrl::U4, &['$']) } else { dq(x, Ctrl::U4, &[]) };
    let media = |ct: &str| if kotlin { format!("{}.toMediaType()", q(ct)) } else { format!("MediaType.parse({})", q(ct)) };
    let ct = s.content_type().unwrap_or("application/octet-stream");
    let body = match &s.body {
        Payload::None => None,
        Payload::Form(fields) => {
            let mut b = if kotlin { "FormBody.Builder()".to_string() } else { "new FormBody.Builder()".to_string() };
            for (k, v) in fields {
                let _ = write!(b, "\n        .add({}, {})", q(k), q(v));
            }
            Some(format!("{b}\n        .build()"))
        }
        Payload::Multipart(fields) => {
            let mut b = if kotlin { "MultipartBody.Builder().setType(MultipartBody.FORM)" } else { "new MultipartBody.Builder().setType(MultipartBody.FORM)" }.to_string();
            for f in fields {
                if f.file {
                    let file = if kotlin {
                        format!("File({}).asRequestBody({})", q(&f.value), media("application/octet-stream"))
                    } else {
                        format!("RequestBody.create(new File({}), {})", q(&f.value), media("application/octet-stream"))
                    };
                    let _ = write!(b, "\n        .addFormDataPart({}, {}, {file})", q(&f.name), q(&file_name(&f.value)));
                } else {
                    let _ = write!(b, "\n        .addFormDataPart({}, {})", q(&f.name), q(&f.value));
                }
            }
            Some(format!("{b}\n        .build()"))
        }
        Payload::File(path) => Some(if kotlin { format!("File({}).asRequestBody({})", q(path), media(ct)) } else { format!("RequestBody.create(new File({}), {})", q(path), media(ct)) }),
        _ => {
            let t = q(&s.text().unwrap_or_default());
            Some(if kotlin { format!("{t}.toRequestBody({})", media(ct)) } else { format!("RequestBody.create({t}, {})", media(ct)) })
        }
    };
    let files = matches!(s.body, Payload::File(_)) || matches!(&s.body, Payload::Multipart(f) if f.iter().any(|f| f.file));
    let multipart_or_form = matches!(s.body, Payload::Form(_) | Payload::Multipart(_));
    let mut o = String::new();
    if kotlin {
        let text = body.is_some() && !multipart_or_form && !matches!(s.body, Payload::File(_));
        let mut imports = vec!["okhttp3.*"];
        if text || files {
            imports.push("okhttp3.MediaType.Companion.toMediaType");
        }
        if text {
            imports.push("okhttp3.RequestBody.Companion.toRequestBody");
        }
        if files {
            imports.extend(["okhttp3.RequestBody.Companion.asRequestBody", "java.io.File"]);
        }
        for i in imports {
            let _ = writeln!(o, "import {i}");
        }
        o.push_str("\nfun main() {\n");
        let _ = writeln!(o, "    val client = OkHttpClient.Builder(){}.build()", if s.follow { "" } else { ".followRedirects(false)" });
        if let Some(b) = &body {
            let _ = writeln!(o, "    val body = {b}");
        }
        let _ = writeln!(o, "    val request = Request.Builder()\n        .url({})", q(s.url.as_str()));
        let _ = writeln!(o, "        .method({}, {})", q(&s.method), if body.is_some() { "body" } else { "null" });
        for (n, v) in s.other_headers() {
            let _ = writeln!(o, "        .addHeader({}, {})", q(n), q(v));
        }
        o.push_str("        .build()\n    client.newCall(request).execute().use { response ->\n        println(response.code)\n        println(response.body?.string())\n    }\n}\n");
    } else {
        o.push_str("import okhttp3.*;\n");
        if files {
            o.push_str("import java.io.File;\n");
        }
        o.push_str("\npublic class Main {\n    public static void main(String[] args) throws Exception {\n");
        let _ = writeln!(o, "        OkHttpClient client = new OkHttpClient.Builder(){}.build();", if s.follow { "" } else { ".followRedirects(false)" });
        if let Some(b) = &body {
            let _ = writeln!(o, "        RequestBody body = {};", b.replace("\n        ", "\n            "));
        }
        let _ = writeln!(o, "        Request request = new Request.Builder()\n            .url({})", q(s.url.as_str()));
        let _ = writeln!(o, "            .method({}, {})", q(&s.method), if body.is_some() { "body" } else { "null" });
        for (n, v) in s.other_headers() {
            let _ = writeln!(o, "            .addHeader({}, {})", q(n), q(v));
        }
        o.push_str("            .build();\n        try (Response response = client.newCall(request).execute()) {\n            System.out.println(response.code());\n            System.out.println(response.body().string());\n        }\n    }\n}\n");
    }
    o
}

fn csharp(s: &Spec) -> String {
    let q = |x: &str| dq(x, Ctrl::U4, &[]);
    let mut o = String::from("using System.Net.Http.Headers;\n\n");
    let handler: Vec<&str> = [
        s.insecure.then_some("ServerCertificateCustomValidationCallback = HttpClientHandler.DangerousAcceptAnyServerCertificateValidator"),
        (!s.follow).then_some("AllowAutoRedirect = false"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if handler.is_empty() {
        o.push_str("using var client = new HttpClient();\n");
    } else {
        let _ = writeln!(o, "using var client = new HttpClient(new HttpClientHandler\n{{\n    {},\n}});", handler.join(",\n    "));
    }
    let _ = writeln!(o, "var request = new HttpRequestMessage(new HttpMethod({}), {});", q(&s.method), q(s.url.as_str()));
    for (n, v) in s.other_headers() {
        let _ = writeln!(o, "request.Headers.TryAddWithoutValidation({}, {});", q(n), q(v));
    }
    match &s.body {
        Payload::None => {}
        Payload::Multipart(fields) => {
            o.push_str("var form = new MultipartFormDataContent();\n");
            for f in fields {
                if f.file {
                    let _ = writeln!(o, "form.Add(new StreamContent(File.OpenRead({})), {}, {});", q(&f.value), q(&f.name), q(&file_name(&f.value)));
                } else {
                    let _ = writeln!(o, "form.Add(new StringContent({}), {});", q(&f.value), q(&f.name));
                }
            }
            o.push_str("request.Content = form;\n");
        }
        Payload::Form(fields) => {
            let pairs: Vec<String> = fields.iter().map(|(k, v)| format!("    new KeyValuePair<string, string>({}, {}),", q(k), q(v))).collect();
            let _ = writeln!(o, "request.Content = new FormUrlEncodedContent(new[]\n{{\n{}\n}});", pairs.join("\n"));
        }
        Payload::File(path) => {
            let _ = writeln!(o, "request.Content = new StreamContent(File.OpenRead({}));", q(path));
        }
        _ => {
            let _ = writeln!(o, "request.Content = new StringContent({});", q(&s.text().unwrap_or_default()));
        }
    }
    if let Some(ct) = s.content_type().filter(|_| !matches!(s.body, Payload::Form(_) | Payload::None)) {
        let _ = writeln!(o, "request.Content.Headers.ContentType = MediaTypeHeaderValue.Parse({});", q(ct));
    }
    o.push_str("using var response = await client.SendAsync(request);\nConsole.WriteLine((int)response.StatusCode);\nConsole.WriteLine(await response.Content.ReadAsStringAsync());\n");
    o
}

fn php_curl(s: &Spec) -> String {
    let mut opts = vec![
        format!("CURLOPT_URL => {}", sq(s.url.as_str())),
        "CURLOPT_RETURNTRANSFER => true".into(),
        format!("CURLOPT_CUSTOMREQUEST => {}", sq(&s.method)),
    ];
    match &s.body {
        Payload::None => {}
        Payload::Multipart(fields) => {
            let items: Vec<String> = fields
                .iter()
                .map(|f| if f.file { format!("        {} => new CURLFile({}),", sq(&f.name), sq(&f.value)) } else { format!("        {} => {},", sq(&f.name), sq(&f.value)) })
                .collect();
            opts.push(format!("CURLOPT_POSTFIELDS => [\n{}\n    ]", items.join("\n")));
        }
        Payload::File(path) => opts.push(format!("CURLOPT_POSTFIELDS => file_get_contents({})", sq(path))),
        _ => opts.push(format!("CURLOPT_POSTFIELDS => {}", sq(&s.text().unwrap_or_default()))),
    }
    if !s.headers.is_empty() {
        let items: Vec<String> = s.headers.iter().map(|(n, v)| format!("        {},", sq(&format!("{n}: {v}")))).collect();
        opts.push(format!("CURLOPT_HTTPHEADER => [\n{}\n    ]", items.join("\n")));
    }
    if let Some((u, p)) = &s.digest {
        opts.push("CURLOPT_HTTPAUTH => CURLAUTH_DIGEST".into());
        opts.push(format!("CURLOPT_USERPWD => {}", sq(&format!("{u}:{p}"))));
    }
    if s.insecure {
        opts.push("CURLOPT_SSL_VERIFYPEER => false".into());
        opts.push("CURLOPT_SSL_VERIFYHOST => 0".into());
    }
    if s.follow {
        opts.push("CURLOPT_FOLLOWLOCATION => true".into());
    }
    let mut o = String::from("<?php\n\n$curl = curl_init();\ncurl_setopt_array($curl, [\n");
    for opt in opts {
        let _ = writeln!(o, "    {opt},");
    }
    o.push_str("]);\n$response = curl_exec($curl);\n$error = curl_error($curl);\n$status = curl_getinfo($curl, CURLINFO_RESPONSE_CODE);\ncurl_close($curl);\n\necho $error ?: $status . \"\\n\" . $response;\n");
    o
}

fn php_guzzle(s: &Spec) -> String {
    let mut opts = Vec::new();
    let skip_type = matches!(s.body, Payload::Form(_));
    let headers: Vec<String> =
        s.headers.iter().filter(|(n, _)| !(skip_type && n.eq_ignore_ascii_case("content-type"))).map(|(n, v)| format!("        {} => {},", sq(n), sq(v))).collect();
    if !headers.is_empty() {
        opts.push(format!("'headers' => [\n{}\n    ]", headers.join("\n")));
    }
    match &s.body {
        Payload::None => {}
        Payload::Form(fields) => {
            let items: Vec<String> = fields.iter().map(|(k, v)| format!("        {} => {},", sq(k), sq(v))).collect();
            opts.push(format!("'form_params' => [\n{}\n    ]", items.join("\n")));
        }
        Payload::Multipart(fields) => {
            let items: Vec<String> = fields
                .iter()
                .map(|f| {
                    if f.file {
                        format!("        ['name' => {}, 'contents' => fopen({}, 'r'), 'filename' => {}],", sq(&f.name), sq(&f.value), sq(&file_name(&f.value)))
                    } else {
                        format!("        ['name' => {}, 'contents' => {}],", sq(&f.name), sq(&f.value))
                    }
                })
                .collect();
            opts.push(format!("'multipart' => [\n{}\n    ]", items.join("\n")));
        }
        Payload::File(path) => opts.push(format!("'body' => fopen({}, 'r')", sq(path))),
        _ => opts.push(format!("'body' => {}", sq(&s.text().unwrap_or_default()))),
    }
    if let Some((u, p)) = &s.digest {
        opts.push(format!("'auth' => [{}, {}, 'digest']", sq(u), sq(p)));
    }
    if s.insecure {
        opts.push("'verify' => false".into());
    }
    if !s.follow {
        opts.push("'allow_redirects' => false".into());
    }
    opts.push("'http_errors' => false".into());
    let mut o = String::from("<?php\n\nrequire 'vendor/autoload.php';\n\n$client = new \\GuzzleHttp\\Client();\n");
    let _ = writeln!(o, "$response = $client->request({}, {}, [", sq(&s.method), sq(s.url.as_str()));
    for opt in opts {
        let _ = writeln!(o, "    {opt},");
    }
    o.push_str("]);\n\necho $response->getStatusCode(), \"\\n\", $response->getBody();\n");
    o
}

fn ruby(s: &Spec) -> String {
    let mut o = String::from("require 'uri'\nrequire 'net/http'\n");
    if s.insecure && s.is_https() {
        o.push_str("require 'openssl'\n");
    }
    let _ = writeln!(o, "\nurl = URI({})\nhttp = Net::HTTP.new(url.host, url.port)", sq(s.url.as_str()));
    if s.is_https() {
        o.push_str("http.use_ssl = true\n");
        if s.insecure {
            o.push_str("http.verify_mode = OpenSSL::SSL::VERIFY_NONE\n");
        }
    }
    let class = match s.method.as_str() {
        "GET" => Some("Get"),
        "POST" => Some("Post"),
        "PUT" => Some("Put"),
        "PATCH" => Some("Patch"),
        "DELETE" => Some("Delete"),
        "HEAD" => Some("Head"),
        "OPTIONS" => Some("Options"),
        _ => None,
    };
    let has_body = !matches!(s.body, Payload::None);
    match class {
        Some(c) => {
            let _ = writeln!(o, "request = Net::HTTP::{c}.new(url)");
        }
        None => {
            let _ = writeln!(o, "request = Net::HTTPGenericRequest.new({}, {has_body}, true, url.request_uri)", sq(&s.method));
        }
    }
    for (n, v) in &s.headers {
        let _ = writeln!(o, "request[{}] = {}", sq(n), sq(v));
    }
    match &s.body {
        Payload::None => {}
        Payload::Multipart(fields) => {
            let items: Vec<String> =
                fields.iter().map(|f| if f.file { format!("  [{}, File.open({})],", sq(&f.name), sq(&f.value)) } else { format!("  [{}, {}],", sq(&f.name), sq(&f.value)) }).collect();
            let _ = writeln!(o, "request.set_form([\n{}\n], 'multipart/form-data')", items.join("\n"));
        }
        Payload::File(path) => {
            let _ = writeln!(o, "request.body = File.binread({})", sq(path));
        }
        _ => {
            let _ = writeln!(o, "request.body = {}", sq(&s.text().unwrap_or_default()));
        }
    }
    o.push_str("\nresponse = http.request(request)\nputs response.code\nputs response.read_body\n");
    o
}

fn swift(s: &Spec) -> String {
    let q = |x: &str| dq(x, Ctrl::Braced, &[]);
    let mut o = String::from("import Foundation\n\n");
    let _ = writeln!(o, "var request = URLRequest(url: URL(string: {})!)\nrequest.httpMethod = {}", q(s.url.as_str()), q(&s.method));
    for (n, v) in &s.headers {
        let _ = writeln!(o, "request.setValue({}, forHTTPHeaderField: {})", q(v), q(n));
    }
    match &s.body {
        Payload::None | Payload::Multipart(_) => {}
        Payload::File(path) => {
            let _ = writeln!(o, "request.httpBody = try Data(contentsOf: URL(fileURLWithPath: {}))", q(path));
        }
        _ => {
            let _ = writeln!(o, "request.httpBody = Data({}.utf8)", q(&s.text().unwrap_or_default()));
        }
    }
    o.push_str("\nlet (data, response) = try await URLSession.shared.data(for: request)\nprint((response as! HTTPURLResponse).statusCode)\nprint(String(decoding: data, as: UTF8.self))\n");
    o
}

fn rust(s: &Spec) -> String {
    let q = |x: &str| dq(x, Ctrl::Braced, &[]);
    let mut o = String::from("// Cargo.toml: reqwest = { version = \"0.12\", features = [\"multipart\"] }, tokio = { version = \"1\", features = [\"full\"] }\n\n");
    o.push_str("#[tokio::main]\nasync fn main() -> Result<(), Box<dyn std::error::Error>> {\n    let client = reqwest::Client::builder()");
    if s.insecure {
        o.push_str("\n        .danger_accept_invalid_certs(true)");
    }
    if !s.follow {
        o.push_str("\n        .redirect(reqwest::redirect::Policy::none())");
    }
    o.push_str("\n        .build()?;\n");
    if let Payload::Multipart(fields) = &s.body {
        o.push_str("    let form = reqwest::multipart::Form::new()");
        for f in fields {
            if f.file {
                let _ = write!(o, "\n        .part({}, reqwest::multipart::Part::bytes(std::fs::read({})?).file_name({}))", q(&f.name), q(&f.value), q(&file_name(&f.value)));
            } else {
                let _ = write!(o, "\n        .text({}, {})", q(&f.name), q(&f.value));
            }
        }
        o.push_str(";\n");
    }
    let _ = write!(o, "    let response = client\n        .request(reqwest::Method::from_bytes({}.as_bytes())?, {})", q(&s.method), q(s.url.as_str()));
    for (n, v) in &s.headers {
        let _ = write!(o, "\n        .header({}, {})", q(n), q(v));
    }
    match &s.body {
        Payload::None => {}
        Payload::Multipart(_) => o.push_str("\n        .multipart(form)"),
        Payload::File(path) => {
            let _ = write!(o, "\n        .body(std::fs::read({})?)", q(path));
        }
        _ => {
            let _ = write!(o, "\n        .body({})", q(&s.text().unwrap_or_default()));
        }
    }
    o.push_str("\n        .send()\n        .await?;\n    println!(\"{}\", response.status().as_u16());\n    println!(\"{}\", response.text().await?);\n    Ok(())\n}\n");
    o
}

/// Dart: single quotes, with `$` and newlines escaped.
fn dart_str(x: &str) -> String {
    let mut out = String::from("'");
    for c in x.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn dart(s: &Spec) -> String {
    let q = dart_str;
    let mut o = String::from("import 'package:http/http.dart' as http;\n");
    if matches!(s.body, Payload::File(_)) {
        o.push_str("import 'dart:io';\n");
    }
    o.push_str("\nvoid main() async {\n");
    let multipart = matches!(s.body, Payload::Multipart(_));
    let class = if multipart { "MultipartRequest" } else { "Request" };
    let _ = writeln!(o, "  final request = http.{class}({}, Uri.parse({}));", q(&s.method), q(s.url.as_str()));
    let skip_type = matches!(s.body, Payload::Form(_));
    let headers: Vec<String> = s.headers.iter().filter(|(n, _)| !(skip_type && n.eq_ignore_ascii_case("content-type"))).map(|(n, v)| format!("    {}: {},", q(n), q(v))).collect();
    if !headers.is_empty() {
        let _ = writeln!(o, "  request.headers.addAll({{\n{}\n  }});", headers.join("\n"));
    }
    match &s.body {
        Payload::None => {}
        Payload::Multipart(fields) => {
            for f in fields {
                if f.file {
                    let _ = writeln!(o, "  request.files.add(await http.MultipartFile.fromPath({}, {}));", q(&f.name), q(&f.value));
                } else {
                    let _ = writeln!(o, "  request.fields[{}] = {};", q(&f.name), q(&f.value));
                }
            }
        }
        Payload::Form(fields) => {
            let items: Vec<String> = fields.iter().map(|(k, v)| format!("    {}: {},", q(k), q(v))).collect();
            let _ = writeln!(o, "  request.bodyFields = {{\n{}\n  }};", items.join("\n"));
        }
        Payload::File(path) => {
            let _ = writeln!(o, "  request.bodyBytes = await File({}).readAsBytes();", q(path));
        }
        _ => {
            let _ = writeln!(o, "  request.body = {};", q(&s.text().unwrap_or_default()));
        }
    }
    o.push_str("  final response = await request.send();\n  print(response.statusCode);\n  print(await response.stream.bytesToString());\n}\n");
    o
}

fn c_libcurl(s: &Spec) -> String {
    let q = |x: &str| dq(x, Ctrl::Octal, &[]);
    let mut o = String::from("#include <stdio.h>\n#include <curl/curl.h>\n\nint main(void) {\n    CURL *curl = curl_easy_init();\n    if (!curl) return 1;\n");
    let _ = writeln!(o, "    curl_easy_setopt(curl, CURLOPT_CUSTOMREQUEST, {});", q(&s.method));
    let _ = writeln!(o, "    curl_easy_setopt(curl, CURLOPT_URL, {});", q(s.url.as_str()));
    o.push_str("    struct curl_slist *headers = NULL;\n");
    for (n, v) in &s.headers {
        let _ = writeln!(o, "    headers = curl_slist_append(headers, {});", q(&format!("{n}: {v}")));
    }
    o.push_str("    curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);\n");
    let mut cleanup = String::new();
    match &s.body {
        Payload::None => {}
        Payload::Multipart(fields) => {
            o.push_str("    curl_mime *mime = curl_mime_init(curl);\n    curl_mimepart *part;\n");
            for f in fields {
                let _ = writeln!(o, "    part = curl_mime_addpart(mime);\n    curl_mime_name(part, {});", q(&f.name));
                if f.file {
                    let _ = writeln!(o, "    curl_mime_filedata(part, {});", q(&f.value));
                } else {
                    let _ = writeln!(o, "    curl_mime_data(part, {}, CURL_ZERO_TERMINATED);", q(&f.value));
                }
            }
            o.push_str("    curl_easy_setopt(curl, CURLOPT_MIMEPOST, mime);\n");
            cleanup.push_str("    curl_mime_free(mime);\n");
        }
        Payload::File(path) => {
            let _ = writeln!(o, "    FILE *body = fopen({}, \"rb\");\n    if (!body) return 1;", q(path));
            o.push_str("    curl_easy_setopt(curl, CURLOPT_UPLOAD, 1L);\n    curl_easy_setopt(curl, CURLOPT_READDATA, body);\n");
            cleanup.push_str("    fclose(body);\n");
        }
        _ => {
            let _ = writeln!(o, "    curl_easy_setopt(curl, CURLOPT_POSTFIELDS, {});", q(&s.text().unwrap_or_default()));
        }
    }
    if let Some((u, p)) = &s.digest {
        let _ = writeln!(o, "    curl_easy_setopt(curl, CURLOPT_HTTPAUTH, CURLAUTH_DIGEST);\n    curl_easy_setopt(curl, CURLOPT_USERPWD, {});", q(&format!("{u}:{p}")));
    }
    if s.insecure {
        o.push_str("    curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, 0L);\n    curl_easy_setopt(curl, CURLOPT_SSL_VERIFYHOST, 0L);\n");
    }
    if s.follow {
        o.push_str("    curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 1L);\n");
    }
    o.push_str("    CURLcode res = curl_easy_perform(curl);\n    if (res != CURLE_OK) {\n        fprintf(stderr, \"%s\\n\", curl_easy_strerror(res));\n    } else {\n        long status;\n        curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &status);\n        fprintf(stderr, \"\\n%ld\\n\", status);\n    }\n");
    o.push_str(&cleanup);
    o.push_str("    curl_slist_free_all(headers);\n    curl_easy_cleanup(curl);\n    return res == CURLE_OK ? 0 : 1;\n}\n");
    o
}

fn raw_http(s: &Spec) -> String {
    let mut o = format!("{} {} HTTP/1.1\r\n", s.method, s.target());
    let host = match s.url.port() {
        Some(p) => format!("{}:{p}", s.url.host_str().unwrap_or_default()),
        None => s.url.host_str().unwrap_or_default().to_string(),
    };
    if s.header("host").is_none() {
        let _ = write!(o, "Host: {host}\r\n");
    }
    for (n, v) in &s.headers {
        let _ = write!(o, "{n}: {v}\r\n");
    }
    let body = match &s.body {
        Payload::None => None,
        Payload::Multipart(fields) => {
            let boundary = "----TsubameDoBoundary";
            let _ = write!(o, "Content-Type: multipart/form-data; boundary={boundary}\r\n");
            let mut b = String::new();
            for f in fields {
                if f.file {
                    let _ = write!(
                        b,
                        "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\r\n<содержимое {}>\r\n",
                        f.name,
                        file_name(&f.value),
                        f.value
                    );
                } else {
                    let _ = write!(b, "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n", f.name, f.value);
                }
            }
            let _ = write!(b, "--{boundary}--\r\n");
            Some(b)
        }
        Payload::File(path) => Some(format!("<содержимое {path}>")),
        Payload::Json(_, t) => Some(t.clone()),
        _ => s.text(),
    };
    if let Some(b) = &body
        && !matches!(s.body, Payload::Multipart(_) | Payload::File(_))
    {
        let _ = write!(o, "Content-Length: {}\r\n", b.len());
    }
    o.push_str("\r\n");
    if let Some(b) = body {
        o.push_str(&b);
    }
    o.replace("\r\n", "\n")
}
