//! What a request checks in its response, and what it keeps from it for other requests.
//!
//! Both read the response with the same short queries:
//!
//! ```text
//! status                    200
//! time                      milliseconds from sending to the last byte
//! size                      body bytes after decompression
//! header Content-Type       the header's value (several joined with ", ")
//! body                      the body as text
//! $.items[0].id             JSONPath (RFC 9535) on the body
//! … count                   length of an array, string or object, or the number of matches
//! ```
//!
//! A check is a query and a condition, one per line:
//!
//! ```text
//! status == 200
//! time < 500
//! header Content-Type contains json
//! $.items count > 0
//! $.user.name == "Котик"
//! $.token exists
//! $.error not exists
//! $.email matches ^[^@]+@[^@]+$
//! ```
//!
//! Conditions: `==`, `!=`, `<`, `<=`, `>`, `>=`, `contains`, `matches`, `exists`, `empty`; `not`
//! before the word ones. The value is JSON (`200`, `"text"`, `true`, `null`, `[1, 2]`) or, when it
//! is not, plain text. `{{variables}}` work anywhere in the line.

use std::io::Read;

use serde::Serialize;
use serde_json::Value;
use serde_json_path::JsonPath;

use crate::http::Response;
use crate::model::{Check, Pair};
use crate::template::{Renderer, Vars};

/// JSONPath reads the body whole; past this a check reports it instead of eating the memory.
const JSON_LIMIT: u64 = 64 << 20;
/// How much of an actual value is shown next to a check.
const SHOWN: usize = 200;

#[derive(Debug, Clone)]
enum Source {
    Status,
    Time,
    Size,
    Header(String),
    Body,
    Path(JsonPath, String),
}

/// Where a value is in the response.
#[derive(Debug, Clone)]
pub struct Query {
    source: Source,
    count: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
    Matches,
    Exists,
    Empty,
}

#[derive(Debug, Clone)]
struct Rule {
    query: Query,
    op: Op,
    not: bool,
    expected: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    /// The line as written.
    pub check: String,
    pub ok: bool,
    /// What the response had, shortened.
    pub actual: Option<String>,
    /// Why it failed, when the reason is not just "the value is different".
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Captured {
    pub name: String,
    pub value: Option<String>,
    pub error: Option<String>,
}

impl Query {
    pub fn parse(text: &str) -> Result<Self, String> {
        let (query, rest) = Self::parse_prefix(text.trim())?;
        if !rest.trim().is_empty() {
            return Err(format!("лишнее после запроса: «{}»", rest.trim()));
        }
        Ok(query)
    }

    /// A query at the start of `text` and what follows it.
    fn parse_prefix(text: &str) -> Result<(Self, &str), String> {
        let text = text.trim_start();
        let (source, rest) = if text.starts_with('$') {
            let end = path_end(text);
            let path = &text[..end];
            let parsed = JsonPath::parse(path).map_err(|e| format!("JSONPath «{path}»: {e}"))?;
            (Source::Path(parsed, path.to_string()), &text[end..])
        } else {
            let (word, rest) = next_word(text);
            match word {
                "status" => (Source::Status, rest),
                "time" => (Source::Time, rest),
                "size" => (Source::Size, rest),
                "body" => (Source::Body, rest),
                "header" => {
                    let rest = rest.trim_start();
                    let (name, rest) = match rest.strip_prefix('"') {
                        Some(quoted) => {
                            let close = quoted.find('"').ok_or("не закрыта кавычка в имени заголовка")?;
                            (&quoted[..close], &quoted[close + 1..])
                        }
                        None => next_word(rest),
                    };
                    if name.is_empty() {
                        return Err("после header нужно имя заголовка".into());
                    }
                    (Source::Header(name.to_string()), rest)
                }
                "" => return Err("пустая строка".into()),
                other => {
                    return Err(format!("непонятно, что проверять: «{other}» (status, time, size, header Имя, body или $.путь)"));
                }
            }
        };
        let (word, after) = next_word(rest);
        let count = matches!(word, "count" | "length");
        Ok((Query { source, count }, if count { after } else { rest }))
    }

    /// The value in the response; `None` when it is not there (no such header or path).
    pub fn eval(&self, r: &Answer) -> Result<Option<Value>, String> {
        let value = match &self.source {
            Source::Status => Some(Value::from(r.response.status)),
            Source::Time => Some(Value::from(r.response.timings.total.as_millis() as u64)),
            Source::Size => Some(Value::from(r.response.body.len())),
            Source::Header(name) => {
                let values: Vec<&str> =
                    r.response.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str()).collect();
                (!values.is_empty()).then(|| Value::String(values.join(", ")))
            }
            Source::Body => Some(Value::String(r.text()?.to_string())),
            Source::Path(path, _) => {
                let nodes = path.query(r.json()?).all();
                match nodes.as_slice() {
                    [] => None,
                    [one] => Some((*one).clone()),
                    many => Some(Value::Array(many.iter().map(|v| (*v).clone()).collect())),
                }
            }
        };
        if !self.count {
            return Ok(value);
        }
        Ok(Some(Value::from(match &value {
            None => 0,
            Some(Value::Array(a)) => a.len(),
            Some(Value::Object(o)) => o.len(),
            Some(Value::String(s)) => s.chars().count(),
            Some(other) => return Err(format!("count бывает у массива, объекта или строки, а здесь {}", short(other))),
        })))
    }

    fn describe(&self) -> String {
        let base = match &self.source {
            Source::Status => "status".into(),
            Source::Time => "time".into(),
            Source::Size => "size".into(),
            Source::Header(name) => format!("заголовка {name}"),
            Source::Body => "тела".into(),
            Source::Path(_, path) => path.clone(),
        };
        if self.count { format!("{base} count") } else { base }
    }
}

/// The end of a JSONPath: the first space outside brackets and quotes.
fn path_end(text: &str) -> usize {
    let mut depth = 0;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            c if c.is_whitespace() && depth <= 0 => return i,
            _ => {}
        }
    }
    text.len()
}

fn next_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    (&text[..end], &text[end..])
}

impl Rule {
    fn parse(text: &str) -> Result<Self, String> {
        let (query, rest) = Query::parse_prefix(text.trim())?;
        let rest = rest.trim_start();
        let (not, rest) = match next_word(rest) {
            ("not", after) => (true, after.trim_start()),
            _ => (false, rest),
        };
        let (op, value) = [
            ("==", Op::Eq),
            ("!=", Op::Ne),
            ("<=", Op::Le),
            (">=", Op::Ge),
            ("<", Op::Lt),
            (">", Op::Gt),
        ]
        .iter()
        .find_map(|&(sign, op)| rest.strip_prefix(sign).map(|v| (op, v)))
        .or_else(|| {
            let (word, after) = next_word(rest);
            let op = match word {
                "contains" => Op::Contains,
                "matches" => Op::Matches,
                "exists" => Op::Exists,
                "empty" => Op::Empty,
                _ => return None,
            };
            Some((op, after))
        })
        .ok_or_else(|| {
            if rest.is_empty() {
                "нет условия: ==, !=, <, >, contains, matches, exists, empty".to_string()
            } else {
                format!("непонятное условие «{rest}»")
            }
        })?;
        if not && !matches!(op, Op::Contains | Op::Matches | Op::Exists | Op::Empty) {
            return Err("not ставится перед contains, matches, exists или empty".into());
        }
        let value = value.trim();
        let needs_value = !matches!(op, Op::Exists | Op::Empty);
        if needs_value && value.is_empty() {
            return Err("после условия нужно значение".into());
        }
        if !needs_value && !value.is_empty() {
            return Err(format!("лишнее после условия: «{value}»"));
        }
        let expected = match op {
            // A pattern is text as written: quotes, if any, are only around it.
            Op::Matches => {
                let pattern = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
                regex::Regex::new(pattern).map_err(|e| format!("регулярное выражение: {e}"))?;
                Value::String(pattern.to_string())
            }
            _ => serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string())),
        };
        if matches!(op, Op::Lt | Op::Le | Op::Gt | Op::Ge) && number(&expected).is_none() {
            return Err(format!("сравнивать на больше-меньше можно только с числом, а не «{value}»"));
        }
        Ok(Rule { query, op, not, expected })
    }

    fn check(&self, actual: Option<&Value>) -> Result<bool, String> {
        if self.op == Op::Exists {
            return Ok(actual.is_some() != self.not);
        }
        let Some(actual) = actual else { return Err(format!("в ответе нет {}", self.query.describe())) };
        let ok = match self.op {
            Op::Eq => same(actual, &self.expected),
            Op::Ne => !same(actual, &self.expected),
            Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                let a = number(actual).ok_or_else(|| format!("{} не число", short(actual)))?;
                let b = number(&self.expected).unwrap_or_default();
                match self.op {
                    Op::Lt => a < b,
                    Op::Le => a <= b,
                    Op::Gt => a > b,
                    _ => a >= b,
                }
            }
            Op::Contains => match actual {
                Value::String(s) => s.contains(&text(&self.expected)),
                Value::Array(items) => items.iter().any(|v| same(v, &self.expected)),
                Value::Object(map) => map.contains_key(&text(&self.expected)),
                other => text(other).contains(&text(&self.expected)),
            },
            Op::Matches => {
                let re = regex::Regex::new(&text(&self.expected)).map_err(|e| e.to_string())?;
                re.is_match(&text(actual))
            }
            Op::Empty => match actual {
                Value::Null => true,
                Value::String(s) => s.is_empty(),
                Value::Array(a) => a.is_empty(),
                Value::Object(o) => o.is_empty(),
                _ => false,
            },
            Op::Exists => unreachable!(),
        };
        Ok(ok != self.not)
    }
}

/// Equal as JSON, with the usual slack of a test: `200 == 200.0`, `"5" == 5`.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(_), Value::Number(_)) => number(a) == number(b),
        (Value::String(s), Value::Number(_) | Value::Bool(_)) | (Value::Number(_) | Value::Bool(_), Value::String(s)) => {
            *s == text(if matches!(a, Value::String(_)) { b } else { a })
        }
        _ => a == b,
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// A value as plain text: strings without quotes, everything else as JSON.
fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn short(v: &Value) -> String {
    let mut s = match v {
        Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    };
    if s.chars().count() > SHOWN {
        s = s.chars().take(SHOWN).collect::<String>() + "…";
    }
    s
}

/// A response with its body read once, as text or JSON, for all the checks.
pub struct Answer<'a> {
    response: &'a Response,
    text: std::cell::OnceCell<Result<String, String>>,
    json: std::cell::OnceCell<Result<Value, String>>,
}

impl<'a> Answer<'a> {
    pub fn new(response: &'a Response) -> Self {
        Answer { response, text: Default::default(), json: Default::default() }
    }

    fn text(&self) -> Result<&str, String> {
        self.text
            .get_or_init(|| {
                let body = &self.response.body;
                if body.len() > JSON_LIMIT {
                    return Err(format!("тело больше {} МБ, его не проверить целиком", JSON_LIMIT >> 20));
                }
                let mut bytes = Vec::new();
                body.reader().and_then(|mut r| r.read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            })
            .as_deref()
            .map_err(Clone::clone)
    }

    fn json(&self) -> Result<&Value, String> {
        self.json
            .get_or_init(|| {
                let text = self.text()?;
                serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("ответ не JSON: {e}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// Runs the enabled checks; a line that does not parse fails with the reason.
pub fn run_checks(checks: &[Check], answer: &Answer, vars: &Vars) -> Vec<CheckResult> {
    checks
        .iter()
        .filter(|c| c.enabled && !c.text.trim().is_empty())
        .map(|c| {
            let failed = |error: String| CheckResult { check: c.text.clone(), ok: false, actual: None, error: Some(error) };
            let mut renderer = Renderer::new(vars);
            let line = renderer.render(&c.text);
            if let Err(e) = renderer.finish() {
                return failed(e.to_string());
            }
            let rule = match Rule::parse(&line) {
                Ok(rule) => rule,
                Err(e) => return failed(e),
            };
            let actual = match rule.query.eval(answer) {
                Ok(actual) => actual,
                Err(e) => return failed(e),
            };
            let shown = actual.as_ref().map(short);
            match rule.check(actual.as_ref()) {
                Ok(ok) => CheckResult { check: c.text.clone(), ok, actual: shown, error: None },
                Err(e) => CheckResult { check: c.text.clone(), ok: false, actual: shown, error: Some(e) },
            }
        })
        .collect()
}

/// Reads the enabled captures: strings as they are, other JSON as JSON text.
pub fn capture(captures: &[Pair], answer: &Answer) -> Vec<Captured> {
    captures
        .iter()
        .filter(|c| c.enabled && !c.name.trim().is_empty())
        .map(|c| {
            let name = c.name.trim().to_string();
            let value = Query::parse(&c.value).and_then(|q| match q.eval(answer)? {
                Some(v) => Ok(text(&v)),
                None => Err(format!("в ответе нет {}", q.describe())),
            });
            match value {
                Ok(value) => Captured { name, value: Some(value), error: None },
                Err(e) => Captured { name, value: None, error: Some(e) },
            }
        })
        .collect()
}

/// Whether a check line is understood, for the editor; `{{variables}}` are taken as written.
pub fn validate(line: &str) -> Result<(), String> {
    let line = line.replace("{{", "").replace("}}", "");
    Rule::parse(&line).map(|_| ())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::http::{ResponseBody, Timings};

    fn response(status: u16, body: &str) -> Response {
        Response {
            status,
            reason: "OK".into(),
            version: "HTTP/1.1".into(),
            headers: vec![("Content-Type".into(), "application/json; charset=utf-8".into()), ("Set-Cookie".into(), "a=1".into()), ("set-cookie".into(), "b=2".into())],
            body: ResponseBody::Memory(bytes::Bytes::from(body.to_string())),
            wire_size: body.len() as u64,
            timings: Timings { total: Duration::from_millis(120), ..Timings::default() },
            url: "http://test/".into(),
            remote_addr: "127.0.0.1:80".parse().unwrap(),
            tls: None,
            redirects: Vec::new(),
        }
    }

    fn results(r: &Response, lines: &[&str], vars: &Vars) -> Vec<(bool, Option<String>, Option<String>)> {
        let checks: Vec<Check> = lines.iter().map(|l| Check::new(*l)).collect();
        run_checks(&checks, &Answer::new(r), vars).into_iter().map(|c| (c.ok, c.actual, c.error)).collect()
    }

    const BODY: &str = r#"{"token":"t0k","user":{"id":7,"name":"Котик","email":"kot@test.ru"},"items":[{"price":10},{"price":25}],"empty":[],"note":null}"#;

    #[test]
    fn passing_checks() {
        let r = response(200, BODY);
        let vars: Vars = [("uid".to_string(), "7".to_string())].into();
        let lines = [
            "status == 200",
            "status < 300",
            "time <= 120",
            "header content-type contains json",
            "header \"Set-Cookie\" == \"a=1, b=2\"",
            "$.token exists",
            "$.error not exists",
            "$.user.id == {{uid}}",
            "$.user.name == \"Котик\"",
            "$.user.name == Котик",
            "$.user.email matches ^[^@]+@test\\.ru$",
            "$.items count == 2",
            "$.items[*].price contains 25",
            "$.items[?@.price > 20] count == 1",
            "$.empty empty",
            "$.note empty",
            "$.user not empty",
            "$.user contains name",
            "body contains Котик",
            "size > 10",
            "$.user count == 3",
        ];
        for (line, (ok, actual, error)) in lines.iter().zip(results(&r, &lines, &vars)) {
            assert!(ok, "{line}: {actual:?} {error:?}");
        }
    }

    #[test]
    fn failing_checks_say_why() {
        let r = response(404, BODY);
        let got = results(&r, &["status == 200", "$.missing == 1", "$.user.name < 3", "$.token", "zzz == 1", "status == {{nope}}", "status not == 1"], &Vars::new());
        assert_eq!(got[0], (false, Some("404".into()), None));
        assert_eq!(got[1].2.as_deref(), Some("в ответе нет $.missing"));
        assert_eq!(got[2].2.as_deref(), Some("\"Котик\" не число"));
        assert!(got[3].2.as_deref().unwrap().starts_with("нет условия"));
        assert!(got[4].2.as_deref().unwrap().starts_with("непонятно, что проверять"));
        assert_eq!(got[5].2.as_deref(), Some("не заданы переменные: nope"));
        assert!(got[6].2.is_some());
        assert!(got.iter().all(|g| !g.0));

        let html = response(200, "<html>");
        let got = results(&html, &["$.a exists", "body contains html"], &Vars::new());
        assert!(got[0].2.as_deref().unwrap().starts_with("ответ не JSON"));
        assert!(got[1].0);
    }

    #[test]
    fn captures_values() {
        let r = response(201, BODY);
        let pairs = vec![
            Pair::new("token", "$.token"),
            Pair::new("uid", "$.user.id"),
            Pair::new("user", "$.user"),
            Pair::new("type", "header Content-Type"),
            Pair::new("code", "status"),
            Pair::new("nope", "$.nope"),
            Pair { name: "off".into(), value: "$.token".into(), enabled: false },
        ];
        let got = capture(&pairs, &Answer::new(&r));
        let values: Vec<(&str, Option<&str>)> = got.iter().map(|c| (c.name.as_str(), c.value.as_deref())).collect();
        assert_eq!(
            values,
            [
                ("token", Some("t0k")),
                ("uid", Some("7")),
                ("user", Some(r#"{"id":7,"name":"Котик","email":"kot@test.ru"}"#)),
                ("type", Some("application/json; charset=utf-8")),
                ("code", Some("201")),
                ("nope", None),
            ]
        );
        assert_eq!(got[5].error.as_deref(), Some("в ответе нет $.nope"));
    }

    #[test]
    fn validates_lines_for_the_editor() {
        assert!(validate("$.id == {{id}}").is_ok());
        assert!(validate("$.items[?@.name == 'a b'] count > 0").is_ok());
        assert!(validate("status ==").is_err());
        assert!(validate("$.[").is_err());
    }
}
