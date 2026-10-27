//! Two responses side by side: stage and prod, now and yesterday.
//!
//! - The status, and the headers that differ; those that change on every response (`Date`,
//!   request ids, tracing) are left out unless asked for.
//! - A JSON body by meaning: each change with its path, `$.items[id=42].price`. Arrays whose
//!   elements all have a unique `id` are matched by it, so one inserted item is one change,
//!   not a shift of every index; the same items in another order are said once. Paths in
//!   `ignore` are not compared: `$.meta.requestId`, `$.items[*].updatedAt`, or a bare key
//!   name (`updatedAt`) anywhere.
//! - Other text line by line, in hunks with context.
//! - Bodies too big for either: where they first differ.

use std::io::{BufReader, Read};

use serde::Serialize;
use serde_json::Value;
use similar::{Algorithm, ChangeTag, TextDiff};

use crate::http::ResponseBody;

/// Bodies up to this size each are compared as JSON.
const JSON_LIMIT: u64 = 32 << 20;
/// And as lines.
const TEXT_LIMIT: u64 = 8 << 20;
/// Changes listed; the rest are counted.
const MAX_CHANGES: usize = 1000;
const MAX_LINES: usize = 5000;
const PREVIEW: usize = 300;

/// Headers that differ from one response to the next whatever the server does.
const VOLATILE: &[&str] = &[
    "date", "age", "expires", "last-modified", "etag", "set-cookie", "content-length", "x-request-id", "request-id", "x-correlation-id",
    "x-trace-id", "traceparent", "tracestate", "x-amzn-requestid", "x-amzn-trace-id", "x-amz-cf-id", "x-amz-cf-pop", "x-amz-request-id",
    "x-amz-id-2", "cf-ray", "x-cache", "x-cache-hits", "x-served-by", "x-timer", "via", "server-timing", "x-runtime", "x-response-time",
    "report-to", "nel", "x-envoy-upstream-service-time",
];

pub struct Side<'a> {
    pub status: u16,
    pub headers: &'a [(String, String)],
    pub body: &'a ResponseBody,
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub ignore: Vec<String>,
    pub all_headers: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub status: [u16; 2],
    /// Only those that differ.
    pub headers: Vec<HeaderDiff>,
    /// Changing-anyway headers that differ and are not shown.
    pub hidden_headers: usize,
    pub body: BodyDiff,
}

impl Comparison {
    /// Nothing that matters differs.
    pub fn same(&self) -> bool {
        self.status[0] == self.status[1]
            && self.headers.is_empty()
            && match &self.body {
                BodyDiff::Same { .. } => true,
                BodyDiff::Json { total, .. } => *total == 0,
                BodyDiff::Text { added, removed, .. } => added + removed == 0,
                BodyDiff::Big { first_difference, .. } => first_difference.is_none(),
            }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HeaderDiff {
    pub name: String,
    pub left: Option<String>,
    pub right: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase", rename_all_fields = "camelCase")]
pub enum BodyDiff {
    /// Byte for byte.
    Same { size: u64 },
    Json { changes: Vec<Change>, total: usize, ignored: usize },
    Text { lines: Vec<Line>, added: usize, removed: usize, truncated: bool },
    Big { sizes: [u64; 2], first_difference: Option<Position> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub path: String,
    pub kind: ChangeKind,
    pub left: Option<String>,
    pub right: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
    /// The same elements in another order.
    Order,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    /// `same`, `added`, `removed`, or `gap` between hunks.
    pub tag: &'static str,
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Position {
    /// From 1.
    pub line: u64,
    pub byte: u64,
}

pub fn compare(left: &Side, right: &Side, options: &Options) -> std::io::Result<Comparison> {
    let (headers, hidden_headers) = headers(left.headers, right.headers, options.all_headers);
    Ok(Comparison { status: [left.status, right.status], headers, hidden_headers, body: bodies(left.body, right.body, &options.ignore)? })
}

fn headers(left: &[(String, String)], right: &[(String, String)], all: bool) -> (Vec<HeaderDiff>, usize) {
    // A header that repeats is compared as all its values together.
    fn grouped(list: &[(String, String)]) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (name, value) in list {
            match out.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                Some((_, v)) => {
                    v.push_str(", ");
                    v.push_str(value);
                }
                None => out.push((name.clone(), value.clone())),
            }
        }
        out
    }
    let (l, r) = (grouped(left), grouped(right));
    let mut names: Vec<&String> = l.iter().map(|(n, _)| n).collect();
    names.extend(r.iter().map(|(n, _)| n).filter(|n| !l.iter().any(|(x, _)| x.eq_ignore_ascii_case(n))));
    let find = |list: &[(String, String)], name: &str| list.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone());
    let mut out = Vec::new();
    let mut hidden = 0;
    for name in names {
        let (a, b) = (find(&l, name), find(&r, name));
        if a == b {
            continue;
        }
        if !all && VOLATILE.contains(&name.to_ascii_lowercase().as_str()) {
            hidden += 1;
            continue;
        }
        out.push(HeaderDiff { name: name.clone(), left: a, right: b });
    }
    (out, hidden)
}

fn bodies(left: &ResponseBody, right: &ResponseBody, ignore: &[String]) -> std::io::Result<BodyDiff> {
    if let Some(at) = first_difference(left, right)? {
        let sizes = [left.len(), right.len()];
        let looks_json = |b: &ResponseBody| b.head(64).is_ok_and(|h| matches!(h.trim_ascii_start().first(), Some(b'{' | b'[')));
        if sizes.iter().all(|s| *s <= JSON_LIMIT)
            && looks_json(left)
            && looks_json(right)
            && let (Ok(a), Ok(b)) = (parse(left), parse(right))
        {
            let patterns: Vec<Vec<Seg>> = ignore.iter().filter(|p| !p.trim().is_empty()).map(|p| pattern(p.trim())).collect();
            let mut walk = Walk { patterns, changes: Vec::new(), total: 0, ignored: 0 };
            walk.value(&mut Vec::new(), &a, &b);
            return Ok(BodyDiff::Json { changes: walk.changes, total: walk.total, ignored: walk.ignored });
        }
        if sizes.iter().all(|s| *s <= TEXT_LIMIT) {
            return Ok(lines(&text(left)?, &text(right)?));
        }
        return Ok(BodyDiff::Big { sizes, first_difference: Some(at) });
    }
    Ok(BodyDiff::Same { size: left.len() })
}

fn parse(body: &ResponseBody) -> std::io::Result<Value> {
    serde_json::from_reader(BufReader::new(body.reader()?)).map_err(std::io::Error::other)
}

fn text(body: &ResponseBody) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    body.reader()?.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Where two bodies first differ, reading both as a stream; `None` when they are the same.
fn first_difference(left: &ResponseBody, right: &ResponseBody) -> std::io::Result<Option<Position>> {
    let (mut a, mut b) = (BufReader::with_capacity(1 << 16, left.reader()?), BufReader::with_capacity(1 << 16, right.reader()?));
    let (mut ba, mut bb) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    let (mut byte, mut line) = (0u64, 1u64);
    loop {
        let na = read_full(&mut a, &mut ba)?;
        let nb = read_full(&mut b, &mut bb)?;
        let n = na.min(nb);
        if let Some(i) = (0..n).find(|&i| ba[i] != bb[i]) {
            line += memchr::memchr_iter(b'\n', &ba[..i]).count() as u64;
            return Ok(Some(Position { line, byte: byte + i as u64 }));
        }
        line += memchr::memchr_iter(b'\n', &ba[..n]).count() as u64;
        byte += n as u64;
        if na != nb {
            return Ok(Some(Position { line, byte }));
        }
        if n == 0 {
            return Ok(None);
        }
    }
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

fn lines(left: &str, right: &str) -> BodyDiff {
    let diff = TextDiff::configure().algorithm(Algorithm::Patience).timeout(std::time::Duration::from_secs(2)).diff_lines(left, right);
    let (mut out, mut added, mut removed, mut truncated) = (Vec::new(), 0, 0, false);
    for (i, group) in diff.grouped_ops(3).iter().enumerate() {
        if i > 0 {
            out.push(Line { tag: "gap", left: None, right: None, text: String::new() });
        }
        for op in group {
            for change in diff.iter_changes(op) {
                let tag = match change.tag() {
                    ChangeTag::Equal => "same",
                    ChangeTag::Insert => {
                        added += 1;
                        "added"
                    }
                    ChangeTag::Delete => {
                        removed += 1;
                        "removed"
                    }
                };
                if out.len() >= MAX_LINES {
                    truncated = true;
                    continue;
                }
                let text = change.value().trim_end_matches(['\n', '\r']).to_string();
                out.push(Line { tag, left: change.old_index().map(|i| i + 1), right: change.new_index().map(|i| i + 1), text });
            }
        }
    }
    BodyDiff::Text { lines: out, added, removed, truncated }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Seg {
    Key(String),
    Index(usize),
    /// An element of an array matched by its `id`.
    Id(String),
    /// In a pattern: any one step.
    Any,
    /// In a pattern: this key anywhere below.
    Anywhere(String),
}

struct Walk {
    patterns: Vec<Vec<Seg>>,
    changes: Vec<Change>,
    total: usize,
    ignored: usize,
}

impl Walk {
    fn value(&mut self, path: &mut Vec<Seg>, a: &Value, b: &Value) {
        if a == b {
            return;
        }
        if self.ignored(path) {
            self.ignored += 1;
            return;
        }
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                for (k, va) in x {
                    path.push(Seg::Key(k.clone()));
                    match y.get(k) {
                        Some(vb) => self.value(path, va, vb),
                        None => self.change(path, ChangeKind::Removed, Some(va), None),
                    }
                    path.pop();
                }
                for (k, vb) in y.iter().filter(|(k, _)| !x.contains_key(*k)) {
                    path.push(Seg::Key(k.clone()));
                    self.change(path, ChangeKind::Added, None, Some(vb));
                    path.pop();
                }
            }
            (Value::Array(x), Value::Array(y)) => match (ids(x), ids(y)) {
                (Some(ix), Some(iy)) => {
                    for (id, va) in ix.iter().zip(x) {
                        path.push(Seg::Id(id.clone()));
                        match iy.iter().position(|i| i == id) {
                            Some(j) => self.value(path, va, &y[j]),
                            None => self.change(path, ChangeKind::Removed, Some(va), None),
                        }
                        path.pop();
                    }
                    for (id, vb) in iy.iter().zip(y).filter(|(id, _)| !ix.contains(id)) {
                        path.push(Seg::Id(id.clone()));
                        self.change(path, ChangeKind::Added, None, Some(vb));
                        path.pop();
                    }
                    // The same ones in another order.
                    let kept_x: Vec<&String> = ix.iter().filter(|i| iy.contains(i)).collect();
                    let kept_y: Vec<&String> = iy.iter().filter(|i| ix.contains(i)).collect();
                    if kept_x != kept_y {
                        let order = |list: &[&String]| Value::String(list.iter().take(20).map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
                        self.change(path, ChangeKind::Order, Some(&order(&kept_x)), Some(&order(&kept_y)));
                    }
                }
                _ => {
                    for (i, va) in x.iter().enumerate() {
                        path.push(Seg::Index(i));
                        match y.get(i) {
                            Some(vb) => self.value(path, va, vb),
                            None => self.change(path, ChangeKind::Removed, Some(va), None),
                        }
                        path.pop();
                    }
                    for (i, vb) in y.iter().enumerate().skip(x.len()) {
                        path.push(Seg::Index(i));
                        self.change(path, ChangeKind::Added, None, Some(vb));
                        path.pop();
                    }
                }
            },
            _ => self.change(path, ChangeKind::Changed, Some(a), Some(b)),
        }
    }

    fn change(&mut self, path: &[Seg], kind: ChangeKind, a: Option<&Value>, b: Option<&Value>) {
        if self.ignored(path) {
            self.ignored += 1;
            return;
        }
        self.total += 1;
        if self.changes.len() < MAX_CHANGES {
            self.changes.push(Change { path: show(path), kind, left: a.map(preview), right: b.map(preview) });
        }
    }

    fn ignored(&self, path: &[Seg]) -> bool {
        self.patterns.iter().any(|p| matches(p, path))
    }
}

/// The `id` of every element when all are objects with a unique scalar `id`.
fn ids(list: &[Value]) -> Option<Vec<String>> {
    if list.is_empty() {
        return None;
    }
    let ids: Vec<String> = list
        .iter()
        .map(|v| match v.get("id")? {
            Value::String(s) => Some(s.clone()),
            n @ Value::Number(_) => Some(n.to_string()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let mut unique = ids.clone();
    unique.sort();
    unique.dedup();
    (unique.len() == ids.len()).then_some(ids)
}

fn preview(v: &Value) -> String {
    let text = v.to_string();
    if text.chars().count() <= PREVIEW {
        return text;
    }
    let cut: String = text.chars().take(PREVIEW).collect();
    format!("{cut}…")
}

/// `$.items[id=42].price`, `$["a key"][0]`.
fn show(path: &[Seg]) -> String {
    let mut out = String::from("$");
    for s in path {
        match s {
            Seg::Key(k) if !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_') => {
                out.push('.');
                out.push_str(k);
            }
            Seg::Key(k) => out.push_str(&format!("[{}]", Value::String(k.clone()))),
            Seg::Index(i) => out.push_str(&format!("[{i}]")),
            Seg::Id(id) => out.push_str(&format!("[id={id}]")),
            Seg::Any => out.push_str("[*]"),
            Seg::Anywhere(k) => out.push_str(&format!("..{k}")),
        }
    }
    out
}

/// `$.meta.requestId`, `$.items[*].updatedAt`, `$.items[0]`, `$..updatedAt` or `updatedAt`.
fn pattern(text: &str) -> Vec<Seg> {
    let Some(rest) = text.strip_prefix('$') else {
        return vec![Seg::Anywhere(text.trim_start_matches('.').to_string())];
    };
    if let Some(key) = rest.strip_prefix("..") {
        return vec![Seg::Anywhere(key.to_string())];
    }
    let mut out = Vec::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '.' => {
                let mut key = String::new();
                while let Some(&c) = chars.peek().filter(|c| **c != '.' && **c != '[') {
                    key.push(c);
                    chars.next();
                }
                out.push(if key == "*" { Seg::Any } else { Seg::Key(key) });
            }
            '[' => {
                let inner: String = chars.by_ref().take_while(|c| *c != ']').collect();
                let inner = inner.trim();
                out.push(if inner == "*" {
                    Seg::Any
                } else if let Ok(i) = inner.parse() {
                    Seg::Index(i)
                } else if let Some(id) = inner.strip_prefix("id=") {
                    Seg::Id(id.trim_matches(['"', '\'']).to_string())
                } else {
                    Seg::Key(inner.trim_matches(['"', '\'']).to_string())
                });
            }
            _ => {}
        }
    }
    out
}

/// A pattern covers its path and everything below it.
fn matches(pattern: &[Seg], path: &[Seg]) -> bool {
    if let [Seg::Anywhere(key)] = pattern {
        return path.iter().any(|s| matches!(s, Seg::Key(k) if k == key));
    }
    pattern.len() <= path.len()
        && pattern.iter().zip(path).all(|(p, s)| match (p, s) {
            (Seg::Any, _) => true,
            (p, s) => p == s,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn body(s: &str) -> ResponseBody {
        ResponseBody::Memory(Bytes::from(s.to_string()))
    }

    fn json(a: &str, b: &str, ignore: &[&str]) -> BodyDiff {
        bodies(&body(a), &body(b), &ignore.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    fn summary(d: &BodyDiff) -> Vec<String> {
        let BodyDiff::Json { changes, .. } = d else { panic!("{d:?}") };
        changes
            .iter()
            .map(|c| format!("{:?} {} {} → {}", c.kind, c.path, c.left.as_deref().unwrap_or("—"), c.right.as_deref().unwrap_or("—")))
            .collect()
    }

    #[test]
    fn json_by_meaning_and_by_id() {
        let stage = r#"{"total": 3, "items": [{"id": 1, "price": 10}, {"id": 2, "price": 20}, {"id": 3, "price": 30}], "meta": {"requestId": "a", "took": 5}}"#;
        let prod = r#"{"items": [{"id": 1, "price": 10}, {"id": 4, "price": 40}, {"id": 2, "price": 25}, {"id": 3, "price": 30}], "total": 4, "meta": {"requestId": "b", "took": 7}, "new": true}"#;
        let d = json(stage, prod, &["$.meta.requestId", "took"]);
        assert_eq!(
            summary(&d),
            [
                "Changed $.total 3 → 4",
                "Changed $.items[id=2].price 20 → 25",
                "Added $.items[id=4] — → {\"id\":4,\"price\":40}",
                "Added $.new — → true",
            ]
        );
        let BodyDiff::Json { total, ignored, .. } = d else { unreachable!() };
        assert_eq!((total, ignored), (4, 2));

        // Key order and spaces do not matter; the same ids in another order are one change.
        assert_eq!(summary(&json(r#"{"a":1,"b":[{"id":"x"},{"id":"y"}]}"#, r#"{ "b": [{"id":"y"},{"id":"x"}], "a": 1 }"#, &[])), ["Order $.b \"x, y\" → \"y, x\""]);
        // Without ids, by index; a type change is a change.
        assert_eq!(summary(&json("[1, 2]", r#"[1, "2", 3]"#, &[])), ["Changed $[1] 2 → \"2\"", "Added $[2] — → 3"]);
        assert_eq!(summary(&json(r#"{"items":[{"id":1,"at":"x"}]}"#, r#"{"items":[{"id":1,"at":"y"}]}"#, &["$.items[*].at"])), Vec::<String>::new());
    }

    #[test]
    fn text_and_same_bodies() {
        let BodyDiff::Text { lines, added, removed, .. } = bodies(&body("a\nb\nc\n"), &body("a\nB\nc\nd\n"), &[]).unwrap() else { panic!() };
        assert_eq!((added, removed), (2, 1));
        let tags: Vec<String> = lines.iter().map(|l| format!("{}{}", &l.tag[..1], l.text)).collect();
        assert_eq!(tags, ["sa", "rb", "aB", "sc", "ad"]);
        assert!(matches!(bodies(&body("{}"), &body("{}"), &[]).unwrap(), BodyDiff::Same { size: 2 }));
        let a = body("one\ntwo\nthree");
        let b = body("one\ntwo\nthreE");
        assert_eq!(first_difference(&a, &b).unwrap(), Some(Position { line: 3, byte: 12 }));
        assert_eq!(first_difference(&a, &body("one\ntwo")).unwrap(), Some(Position { line: 2, byte: 7 }));
    }

    #[test]
    fn headers_without_the_volatile_ones() {
        let a = vec![("Date".into(), "Mon".into()), ("Content-Type".into(), "application/json".into()), ("X-Version".into(), "1".into())];
        let b = vec![("date".into(), "Tue".into()), ("content-type".into(), "application/json".into()), ("Cache-Control".into(), "no-store".into())];
        let (diff, hidden) = headers(&a, &b, false);
        let names: Vec<(&str, Option<&str>, Option<&str>)> = diff.iter().map(|h| (h.name.as_str(), h.left.as_deref(), h.right.as_deref())).collect();
        assert_eq!(names, [("X-Version", Some("1"), None), ("Cache-Control", None, Some("no-store"))]);
        assert_eq!(hidden, 1);
        assert_eq!(headers(&a, &b, true).0.len(), 3);
    }
}
