//! curl commands in and out: what browsers' "Copy as cURL", API docs and colleagues hand over
//! becomes an editable request, and a request goes back as a command anyone can run.

use base64::Engine;

use crate::model::{Auth, Body, Pair, Part, Request};
use crate::prepare::Prepared;
use crate::template::Vars;
use crate::{Error, Result};

/// Reads a bash-style `curl …` command (line continuations, '…', "…", $'…').
pub fn parse(command: &str) -> Result<Request> {
    let words = split(command)?;
    let mut words = words.into_iter().peekable();
    match words.next().as_deref() {
        Some("curl") => {}
        _ => return Err(Error::Invalid("это не команда curl: она должна начинаться со слова curl".into())),
    }

    let mut cmd = Command::default();
    while let Some(word) = words.next() {
        if word == "--" {
            cmd.url = cmd.url.or(words.next());
            continue;
        }
        if let Some(long) = word.strip_prefix("--") {
            let value = if takes_value_long(long) { Some(words.next().ok_or_else(|| missing(&word))?) } else { None };
            cmd.apply_long(long, value)?;
        } else if word.len() > 1 && word.starts_with('-') {
            let flags: Vec<char> = word[1..].chars().collect();
            for (i, flag) in flags.iter().enumerate() {
                if takes_value_short(*flag) {
                    let attached: String = flags[i + 1..].iter().collect();
                    let value = if attached.is_empty() { words.next().ok_or_else(|| missing(&word))? } else { attached };
                    cmd.apply_short(*flag, Some(value))?;
                    break;
                }
                cmd.apply_short(*flag, None)?;
            }
        } else if cmd.url.is_none() {
            cmd.url = Some(word);
        }
    }
    cmd.into_request()
}

fn missing(flag: &str) -> Error {
    Error::Invalid(format!("у {flag} нет значения"))
}

fn takes_value_short(flag: char) -> bool {
    "XHdFubAeoxETwmrCKYyzQ".contains(flag)
}

fn takes_value_long(name: &str) -> bool {
    matches!(
        name,
        "request" | "header" | "data" | "data-raw" | "data-binary" | "data-ascii" | "data-urlencode" | "json" | "form"
            | "form-string" | "user" | "cookie" | "user-agent" | "referer" | "url" | "output" | "proxy" | "max-time"
            | "connect-timeout" | "retry" | "cert" | "key" | "cacert" | "capath" | "resolve" | "write-out"
            | "limit-rate" | "oauth2-bearer" | "proxy-user" | "upload-file" | "range" | "config" | "interface"
            | "max-redirs" | "cookie-jar" | "dump-header" | "trace" | "trace-ascii" | "connect-to" | "aws-sigv4"
            | "pass" | "cert-type" | "key-type" | "ciphers" | "noproxy" | "retry-delay" | "retry-max-time"
    )
}

#[derive(Default)]
struct Command {
    method: Option<String>,
    url: Option<String>,
    headers: Vec<(String, String)>,
    data: Vec<String>,
    /// `@file` given to `-d`/`--data-binary`.
    data_file: Option<String>,
    json: bool,
    form: Vec<Part>,
    user: Option<String>,
    bearer: Option<String>,
    get: bool,
    head: bool,
    insecure: bool,
    max_time_ms: Option<u64>,
}

impl Command {
    fn apply_short(&mut self, flag: char, value: Option<String>) -> Result<()> {
        let long = match flag {
            'X' => "request",
            'H' => "header",
            'd' => "data",
            'F' => "form",
            'u' => "user",
            'b' => "cookie",
            'A' => "user-agent",
            'e' => "referer",
            'G' => "get",
            'I' => "head",
            'k' => "insecure",
            'm' => "max-time",
            // Output, verbosity, redirects and the rest change nothing in the request.
            _ => return Ok(()),
        };
        self.apply_long(long, value)
    }

    fn apply_long(&mut self, name: &str, value: Option<String>) -> Result<()> {
        let value = value.unwrap_or_default();
        match name {
            "request" => self.method = Some(value.to_ascii_uppercase()),
            "url" => self.url = Some(value),
            "header" => {
                if let Some((n, v)) = value.split_once(':') {
                    self.headers.push((n.trim().to_string(), v.trim().to_string()));
                } else if let Some(n) = value.strip_suffix(';') {
                    // `-H 'X-Empty;'` sends an empty header.
                    self.headers.push((n.trim().to_string(), String::new()));
                }
            }
            "data" | "data-ascii" | "data-binary" | "data-raw" | "json" => {
                if name == "json" {
                    self.json = true;
                }
                match value.strip_prefix('@') {
                    Some(path) if name != "data-raw" => self.data_file = Some(path.to_string()),
                    _ => self.data.push(value),
                }
            }
            "data-urlencode" => {
                let encode = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
                self.data.push(match value.split_once('=') {
                    Some((n, v)) => format!("{n}={}", encode(v)),
                    None => encode(&value),
                });
            }
            "form" | "form-string" => {
                let (n, v) = value.split_once('=').unwrap_or((&value, ""));
                let file = if name == "form" { v.strip_prefix('@').or_else(|| v.strip_prefix('<')) } else { None };
                // `@photo.png;type=image/png`: options after the path are curl's business.
                let file = file.map(|f| f.split(';').next().unwrap_or(f).to_string());
                self.form.push(Part {
                    name: n.to_string(),
                    value: file.clone().unwrap_or_else(|| v.to_string()),
                    is_file: file.is_some(),
                    enabled: true,
                });
            }
            "user" => self.user = Some(value),
            "oauth2-bearer" => self.bearer = Some(value),
            "cookie" if value.contains('=') => self.headers.push(("Cookie".into(), value)),
            "user-agent" => self.headers.push(("User-Agent".into(), value)),
            "referer" => self.headers.push(("Referer".into(), value)),
            "get" => self.get = true,
            "head" => self.head = true,
            "insecure" => self.insecure = true,
            "max-time" => {
                let secs: f64 = value.parse().map_err(|_| Error::Invalid(format!("--max-time: «{value}» не число")))?;
                self.max_time_ms = Some((secs * 1000.0) as u64);
            }
            _ => {}
        }
        Ok(())
    }

    fn into_request(mut self) -> Result<Request> {
        let url = self.url.take().ok_or_else(|| Error::Invalid("в команде нет адреса".into()))?;
        let (url, mut query) = split_query(&url);
        let mut request = Request { url, ..Request::default() };
        let header = |headers: &[(String, String)], name: &str| {
            headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone())
        };
        let drop_header = |headers: &mut Vec<(String, String)>, name: &str| headers.retain(|(n, _)| !n.eq_ignore_ascii_case(name));

        // Auth headers become the auth tab, so the token can turn into a {{variable}}.
        if let Some(value) = header(&self.headers, "authorization") {
            let (scheme, rest) = value.split_once(' ').unwrap_or((&value, ""));
            if scheme.eq_ignore_ascii_case("bearer") {
                request.auth = Auth::Bearer { token: rest.trim().to_string() };
                drop_header(&mut self.headers, "authorization");
            } else if scheme.eq_ignore_ascii_case("basic")
                && let Some(decoded) = base64::engine::general_purpose::STANDARD.decode(rest.trim()).ok().and_then(|b| String::from_utf8(b).ok())
            {
                let (username, password) = decoded.split_once(':').unwrap_or((&decoded, ""));
                request.auth = Auth::Basic { username: username.into(), password: password.into() };
                drop_header(&mut self.headers, "authorization");
            }
        }
        if let Some(user) = &self.user {
            let (username, password) = user.split_once(':').unwrap_or((user, ""));
            request.auth = Auth::Basic { username: username.into(), password: password.into() };
        }
        if let Some(token) = &self.bearer {
            request.auth = Auth::Bearer { token: token.clone() };
        }

        let data = (!self.data.is_empty()).then(|| self.data.join("&"));
        let content_type = header(&self.headers, "content-type").map(|c| c.to_ascii_lowercase());
        let mime = content_type.as_deref().map(|c| c.split(';').next().unwrap_or("").trim().to_string());
        if self.get {
            if let Some(data) = &data {
                query.extend(url::form_urlencoded::parse(data.as_bytes()).map(|(n, v)| Pair::new(n, v)));
            }
        } else if !self.form.is_empty() {
            request.body = Body::Multipart { parts: std::mem::take(&mut self.form) };
            drop_header(&mut self.headers, "content-type");
        } else if let Some(path) = self.data_file.take() {
            request.body = Body::File { path, content_type: header(&self.headers, "content-type") };
            drop_header(&mut self.headers, "content-type");
        } else if let Some(data) = data {
            let json = self.json || mime.as_deref().is_some_and(|m| m == "application/json" || m.ends_with("+json"));
            if json {
                request.body = Body::Json { content: data };
                if mime.as_deref() == Some("application/json") && content_type.as_deref() == mime.as_deref() {
                    drop_header(&mut self.headers, "content-type");
                }
                if self.json && header(&self.headers, "accept").is_none() {
                    self.headers.push(("Accept".into(), "application/json".into()));
                }
            } else if matches!(mime.as_deref(), None | Some("application/x-www-form-urlencoded")) && looks_like_form(&data) {
                let fields = url::form_urlencoded::parse(data.as_bytes()).map(|(n, v)| Pair::new(n, v)).collect();
                request.body = Body::Form { fields };
                drop_header(&mut self.headers, "content-type");
            } else {
                request.body = Body::Text {
                    content: data,
                    content_type: Some(header(&self.headers, "content-type").unwrap_or_else(|| "application/x-www-form-urlencoded".into())),
                };
                drop_header(&mut self.headers, "content-type");
            }
        }

        let has_body = !matches!(request.body, Body::None);
        request.method = match self.method {
            Some(m) => m,
            None if self.head => "HEAD".into(),
            None if has_body => "POST".into(),
            None => "GET".into(),
        };
        request.query = query;
        request.headers = self.headers.into_iter().map(|(n, v)| Pair::new(n, v)).collect();
        if self.insecure {
            request.options.verify_tls = Some(false);
        }
        request.options.timeout_ms = self.max_time_ms;
        Ok(request)
    }
}

/// `k=v&k2=v2` that survives a round trip: anything else stays raw text.
fn looks_like_form(data: &str) -> bool {
    !data.is_empty()
        && data.split('&').all(|piece| {
            let (name, _) = piece.split_once('=').unwrap_or((piece, ""));
            !name.is_empty() && piece.contains('=') && !piece.contains(char::is_whitespace)
        })
}

/// The address without its query, and the query as editable pairs. `{{variables}}` in the
/// address must survive, so this is plain text work, not URL parsing.
fn split_query(url: &str) -> (String, Vec<Pair>) {
    let url = url.split('#').next().unwrap_or(url);
    match url.split_once('?') {
        Some((base, query)) if !query.is_empty() => {
            let pairs = query
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let (n, v) = p.split_once('=').unwrap_or((p, ""));
                    let decode = |s: &str| {
                        url::form_urlencoded::parse(format!("x={s}").as_bytes()).next().map(|(_, v)| v.into_owned()).unwrap_or_default()
                    };
                    Pair::new(decode(n), decode(v))
                })
                .collect();
            (base.to_string(), pairs)
        }
        _ => (url.trim_end_matches('?').to_string(), Vec::new()),
    }
}

/// Splits like bash: quotes, `$'…'` escapes, backslash line continuations.
fn split(command: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    let unclosed = || Error::Invalid("в команде не закрыта кавычка".into());
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('\n') => {}
                Some('\r') if chars.peek() == Some(&'\n') => {
                    chars.next();
                }
                Some(next) => {
                    word.push(next);
                    in_word = true;
                }
                None => {}
            },
            '\'' => {
                in_word = true;
                loop {
                    match chars.next().ok_or_else(unclosed)? {
                        '\'' => break,
                        ch => word.push(ch),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next().ok_or_else(unclosed)? {
                        '"' => break,
                        '\\' => match chars.next().ok_or_else(unclosed)? {
                            ch @ ('"' | '\\' | '$' | '`') => word.push(ch),
                            '\n' => {}
                            ch => {
                                word.push('\\');
                                word.push(ch);
                            }
                        },
                        ch => word.push(ch),
                    }
                }
            }
            '$' if chars.peek() == Some(&'\'') => {
                chars.next();
                in_word = true;
                loop {
                    match chars.next().ok_or_else(unclosed)? {
                        '\'' => break,
                        '\\' => match chars.next().ok_or_else(unclosed)? {
                            'n' => word.push('\n'),
                            't' => word.push('\t'),
                            'r' => word.push('\r'),
                            'x' => word.push(hex_char(&mut chars, 2)),
                            'u' => word.push(hex_char(&mut chars, 4)),
                            ch => word.push(ch),
                        },
                        ch => word.push(ch),
                    }
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

fn hex_char(chars: &mut std::iter::Peekable<std::str::Chars>, max: usize) -> char {
    let mut digits = String::new();
    while digits.len() < max && chars.peek().is_some_and(|c| c.is_ascii_hexdigit()) {
        digits.push(chars.next().unwrap());
    }
    u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32).unwrap_or('\u{fffd}')
}

/// A shell-ready command for the prepared request: variables substituted, headers as sent.
/// Multipart and file bodies stay as `-F`/`@file`, so the command stays readable.
pub fn to_curl(prepared: &Prepared, body: &Body, vars: &Vars) -> String {
    crate::codegen::generate("curl", &crate::codegen::Spec::new(prepared, body, vars)).expect("curl is a known language")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ApiKeyLocation;
    use crate::prepare::prepare;

    #[test]
    fn reads_a_browser_copy() {
        let chrome = r#"curl 'https://api.level.travel/orders?page=2&city=%D0%A1%D0%BE%D1%87%D0%B8' \
  -H 'accept: application/json' \
  -H 'authorization: Bearer abc.def' \
  -H 'content-type: application/json' \
  --data-raw $'{"name":"Кот \'Матроскин\'","n":1}' \
  --compressed"#;
        let r = parse(chrome).unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.url, "https://api.level.travel/orders");
        assert_eq!(r.query, vec![Pair::new("page", "2"), Pair::new("city", "Сочи")]);
        assert_eq!(r.headers, vec![Pair::new("accept", "application/json")]);
        assert_eq!(r.auth, Auth::Bearer { token: "abc.def".into() });
        assert_eq!(r.body, Body::Json { content: r#"{"name":"Кот 'Матроскин'","n":1}"#.into() });
    }

    #[test]
    fn reads_forms_users_and_flags() {
        let r = parse("curl -u kot:мяу -d name=Kot -d 'age=3' -XPUT -k -m 2.5 http://x.test/a").unwrap();
        assert_eq!(r.method, "PUT");
        assert_eq!(r.auth, Auth::Basic { username: "kot".into(), password: "мяу".into() });
        assert_eq!(r.body, Body::Form { fields: vec![Pair::new("name", "Kot"), Pair::new("age", "3")] });
        assert_eq!((r.options.verify_tls, r.options.timeout_ms), (Some(false), Some(2500)));

        let r = parse("curl -sSL -G https://x.test/s --data-urlencode 'q=кот и пёс' -H \"X-Id: {{id}}\"").unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.query, vec![Pair::new("q", "кот и пёс")]);
        assert_eq!(r.headers, vec![Pair::new("X-Id", "{{id}}")]);

        let r = parse("curl -F title=Кот -F 'photo=@kot.png;type=image/png' https://x.test/up").unwrap();
        assert_eq!(
            r.body,
            Body::Multipart {
                parts: vec![
                    Part { name: "title".into(), value: "Кот".into(), is_file: false, enabled: true },
                    Part { name: "photo".into(), value: "kot.png".into(), is_file: true, enabled: true },
                ]
            }
        );

        let r = parse("curl --json '{\"a\":1}' https://x.test").unwrap();
        assert!(matches!(r.body, Body::Json { .. }));
        assert_eq!(r.headers, vec![Pair::new("Accept", "application/json")]);

        let r = parse("curl -X POST https://x.test -H 'Content-Type: text/csv' --data-binary @rows.csv").unwrap();
        assert_eq!(r.body, Body::File { path: "rows.csv".into(), content_type: Some("text/csv".into()) });
        assert!(r.headers.is_empty());

        assert!(parse("wget https://x").is_err());
        assert!(parse("curl -H 'oops").is_err());
        assert!(parse("curl -s").is_err());
    }

    #[test]
    fn writes_a_command_that_reads_back() {
        let request = Request {
            method: "POST".into(),
            url: "{{base}}/orders".into(),
            query: vec![Pair::new("dry", "1")],
            headers: vec![Pair::new("X-Note", "it's {{who}}")],
            auth: Auth::ApiKey { key: "X-Key".into(), value: "k1".into(), location: ApiKeyLocation::Header },
            body: Body::Json { content: "{\"a\": \"кот\"}".into() },
            ..Request::default()
        };
        let vars: Vars = [("base", "https://x.test"), ("who", "кот")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let prepared = prepare(&request, &vars, std::path::Path::new(".")).unwrap();
        let command = to_curl(&prepared, &request.body, &vars);
        assert_eq!(
            command,
            "curl \\\n  'https://x.test/orders?dry=1' \\\n  -H 'X-Note: it'\\''s кот' \\\n  -H 'X-Key: k1' \\\n  -H 'Content-Type: application/json' \\\n  --data-raw '{\"a\": \"кот\"}' \\\n  -L"
        );
        let back = parse(&command).unwrap();
        assert_eq!(back.method, "POST");
        assert_eq!(back.url, "https://x.test/orders");
        assert_eq!(back.body, request.body);
    }
}
