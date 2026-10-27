//! Text views of a response body for the viewer, built for bodies of hundreds of megabytes.
//!
//! The interface never gets the whole body: it asks for the lines it shows. A view is the
//! body (raw) or its pretty-printed JSON (written to a temporary file the same way big bodies
//! are), plus a line index of 2 bytes per line and the fold ranges of JSON objects and arrays.
//! Lines longer than [`MAX_LINE`] bytes are cut into several, so minified JSON on one line
//! still scrolls.

use std::io::{self, Write};
use std::path::PathBuf;

use bytes::Bytes;

use crate::http::ResponseBody;
use crate::http::body::Sink;

pub const MAX_LINE: usize = 4096;
const CHECKPOINT: usize = 1024;
/// Search stops counting here; nobody reads further, and it keeps a pathological query fast.
pub const MAX_MATCHES: usize = 1_000_000;
const MAX_MATCH_LINES: usize = 100_000;

/// Line lengths (with their `\n`), and the offset of every 1024th line.
#[derive(Default)]
struct LineIndex {
    lens: Vec<u16>,
    checkpoints: Vec<u64>,
    current: usize,
    offset: u64,
}

impl LineIndex {
    #[inline]
    fn byte(&mut self, b: u8) {
        // A long line is cut before a character starts, never inside one.
        if self.current >= MAX_LINE && b & 0xC0 != 0x80 && b != b'\n' {
            self.end_line();
        }
        self.current += 1;
        if b == b'\n' {
            self.end_line();
        }
    }

    fn feed(&mut self, data: &[u8]) {
        let mut rest = data;
        // Fast path for ordinary lines: jump from newline to newline.
        while let Some(pos) = memchr::memchr(b'\n', rest) {
            if self.current + pos < MAX_LINE {
                self.current += pos + 1;
                self.end_line();
            } else {
                rest[..=pos].iter().for_each(|&b| self.byte(b));
            }
            rest = &rest[pos + 1..];
        }
        rest.iter().for_each(|&b| self.byte(b));
    }

    fn end_line(&mut self) {
        if self.lens.len().is_multiple_of(CHECKPOINT) {
            self.checkpoints.push(self.offset);
        }
        self.lens.push(self.current as u16);
        self.offset += self.current as u64;
        self.current = 0;
    }

    fn finish(&mut self) {
        if self.current > 0 || self.lens.is_empty() {
            self.end_line();
        }
        self.lens.shrink_to_fit();
        self.checkpoints.shrink_to_fit();
    }

    /// The line being written now.
    fn line(&self) -> u32 {
        self.lens.len() as u32
    }

    fn start(&self, line: usize) -> u64 {
        let block = line / CHECKPOINT;
        let from = block * CHECKPOINT;
        self.checkpoints[block] + self.lens[from..line].iter().map(|&l| l as u64).sum::<u64>()
    }
}

enum Data {
    Memory(Bytes),
    Mapped(memmap2::Mmap),
}

impl Data {
    fn of(body: &ResponseBody) -> io::Result<Self> {
        Ok(match body {
            ResponseBody::Memory(b) => Data::Memory(b.clone()),
            // An empty file cannot be mapped.
            ResponseBody::File { len: 0, .. } => Data::Memory(Bytes::new()),
            ResponseBody::File { path, .. } => {
                let file = std::fs::File::open(path.as_ref() as &std::path::Path)?;
                // SAFETY: the file is our private temporary file; nobody truncates it while mapped.
                Data::Mapped(unsafe { memmap2::Mmap::map(&file)? })
            }
        })
    }

    fn bytes(&self) -> &[u8] {
        match self {
            Data::Memory(b) => b,
            Data::Mapped(m) => m,
        }
    }
}

pub struct TextView {
    data: Data,
    index: LineIndex,
    /// `(open, close)` lines of every multi-line object or array, sorted by `open`.
    folds: Vec<(u32, u32)>,
    pub pretty: bool,
    // Keeps the temporary files alive as long as the view is.
    _body: ResponseBody,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// Matches counted, up to [`MAX_MATCHES`].
    pub total: usize,
    /// Lines with a match, in order, at most 100 000.
    pub lines: Vec<u32>,
}

impl TextView {
    /// The body as it came.
    pub fn raw(body: &ResponseBody) -> io::Result<Self> {
        let data = Data::of(body)?;
        let mut index = LineIndex::default();
        index.feed(data.bytes());
        index.finish();
        Ok(TextView { data, index, folds: Vec::new(), pretty: false, _body: body.clone() })
    }

    /// The body as indented JSON; `Ok(Err(reason))` when it is not JSON.
    pub fn pretty_json(body: &ResponseBody, spill_threshold: usize, spill_dir: PathBuf) -> io::Result<Result<Self, String>> {
        let source = Data::of(body)?;
        let mut out = Formatter {
            sink: Sink::new(spill_threshold, spill_dir),
            index: LineIndex::default(),
            folds: Vec::new(),
            error: None,
        };
        if let Err(reason) = out.format(source.bytes()) {
            return Ok(Err(reason));
        }
        if let Some(e) = out.error {
            return Err(e);
        }
        out.index.finish();
        out.folds.sort_unstable();
        out.folds.shrink_to_fit();
        let formatted = out.sink.finish()?;
        Ok(Ok(TextView { data: Data::of(&formatted)?, index: out.index, folds: out.folds, pretty: true, _body: formatted }))
    }

    pub fn line_count(&self) -> usize {
        self.index.lens.len()
    }

    pub fn len(&self) -> u64 {
        self.data.bytes().len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The line without its line break.
    pub fn line(&self, n: usize) -> &[u8] {
        if n >= self.line_count() {
            return &[];
        }
        let start = self.index.start(n) as usize;
        let mut line = &self.data.bytes()[start..start + self.index.lens[n] as usize];
        if let Some(rest) = line.strip_suffix(b"\n") {
            line = rest.strip_suffix(b"\r").unwrap_or(rest);
        }
        line
    }

    /// Where the object or array opened on this line closes.
    pub fn fold_end(&self, line: u32) -> Option<u32> {
        let i = self.folds.partition_point(|&(open, _)| open < line);
        self.folds.get(i).filter(|&&(open, _)| open == line).map(|&(_, close)| close)
    }

    /// The ranges directly inside the top-level object or array, for "fold all"; `None` when
    /// there are more than `limit` (folding two million rows into two million rows helps
    /// nobody).
    pub fn top_folds(&self, limit: usize) -> Option<Vec<(u32, u32)>> {
        let last = self.line_count().saturating_sub(1) as u32;
        let mut out = Vec::new();
        let mut end = 0;
        for &(open, close) in &self.folds {
            if (open == 0 && close == last) || (open <= end && !out.is_empty()) {
                continue;
            }
            if out.len() == limit {
                return None;
            }
            out.push((open, close));
            end = close;
        }
        Some(out)
    }

    pub fn search(&self, matcher: &Matcher) -> SearchResult {
        let mut result = SearchResult { total: 0, lines: Vec::new() };
        let mut line = 0usize;
        let mut line_end = self.index.lens.first().map_or(0, |&l| l as u64);
        for start in matcher.find_iter(self.data.bytes()) {
            let start = start as u64;
            while start >= line_end && line + 1 < self.line_count() {
                line += 1;
                line_end += self.index.lens[line] as u64;
            }
            result.total += 1;
            if result.lines.last() != Some(&(line as u32)) && result.lines.len() < MAX_MATCH_LINES {
                result.lines.push(line as u32);
            }
            if result.total >= MAX_MATCHES {
                break;
            }
        }
        result
    }

    /// The whole view, for copying or saving the pretty text.
    pub fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(self.data.bytes())
    }
}

/// What the user searches for, in any case by default (Unicode: «Москва» finds «МОСКВА»).
/// The regex engine's literal search beat `memmem` on Cyrillic, so it serves both modes.
pub struct Matcher(regex::bytes::Regex);

impl Matcher {
    pub fn new(query: &str, case_sensitive: bool) -> Option<Self> {
        if query.is_empty() {
            return None;
        }
        regex::bytes::RegexBuilder::new(&regex::escape(query))
            .case_insensitive(!case_sensitive)
            .build()
            .ok()
            .map(Matcher)
    }

    fn find_iter<'h>(&'h self, haystack: &'h [u8]) -> impl Iterator<Item = usize> + 'h {
        self.0.find_iter(haystack).map(|m| m.start())
    }

    /// Byte ranges of the matches in one line.
    pub fn ranges(&self, line: &[u8]) -> Vec<(usize, usize)> {
        self.0.find_iter(line).map(|m| (m.start(), m.end())).collect()
    }
}

/// A streaming JSON pretty-printer: two-space indent, like `jq`. It checks brackets, strings
/// and scalars, not every rule of JSON; anything it does not understand means "not JSON".
struct Formatter {
    sink: Sink,
    index: LineIndex,
    folds: Vec<(u32, u32)>,
    error: Option<io::Error>,
}

impl Formatter {
    fn put(&mut self, bytes: &[u8]) {
        if self.error.is_none()
            && let Err(e) = self.sink.write_all(bytes)
        {
            self.error = Some(e);
        }
        self.index.feed(bytes);
    }

    fn newline(&mut self, depth: usize) {
        const SPACES: &[u8] = &[b' '; 128];
        self.put(b"\n");
        let mut left = depth * 2;
        while left > 0 {
            let n = left.min(SPACES.len());
            self.put(&SPACES[..n]);
            left -= n;
        }
    }

    fn format(&mut self, src: &[u8]) -> Result<(), String> {
        let src = src.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(src);
        let ws = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n');
        let mut stack: Vec<(u8, u32)> = Vec::new();
        let mut seen_value = false;
        let mut i = 0;
        while i < src.len() {
            let b = src[i];
            if stack.is_empty() && seen_value && !ws(b) {
                return Err("после JSON есть ещё текст".into());
            }
            match b {
                b' ' | b'\t' | b'\r' | b'\n' => i += 1,
                b'"' => {
                    let mut j = i + 1;
                    loop {
                        let Some(pos) = memchr::memchr2(b'"', b'\\', &src[j..]) else {
                            return Err("строка не закрыта".into());
                        };
                        if src[j + pos] == b'\\' {
                            j += pos + 2;
                        } else {
                            j += pos + 1;
                            break;
                        }
                    }
                    let j = j.min(src.len());
                    self.put(&src[i..j]);
                    i = j;
                    seen_value = true;
                }
                b'{' | b'[' => {
                    let close = if b == b'{' { b'}' } else { b']' };
                    let mut k = i + 1;
                    while k < src.len() && ws(src[k]) {
                        k += 1;
                    }
                    if src.get(k) == Some(&close) {
                        self.put(&[b, close]);
                        i = k + 1;
                        seen_value = true;
                    } else {
                        self.put(&[b]);
                        stack.push((close, self.index.line()));
                        self.newline(stack.len());
                        i += 1;
                    }
                }
                b'}' | b']' => {
                    let Some((want, open)) = stack.pop() else { return Err("лишняя закрывающая скобка".into()) };
                    if want != b {
                        return Err("скобки не совпадают".into());
                    }
                    self.newline(stack.len());
                    self.folds.push((open, self.index.line()));
                    self.put(&[b]);
                    i += 1;
                    seen_value = true;
                }
                b',' if !stack.is_empty() => {
                    self.put(b",");
                    self.newline(stack.len());
                    i += 1;
                }
                b':' if stack.last().is_some_and(|&(c, _)| c == b'}') => {
                    self.put(b": ");
                    i += 1;
                }
                _ => {
                    let start = i;
                    while i < src.len() && matches!(src[i], b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'+' | b'-' | b'.') {
                        i += 1;
                    }
                    if i == start {
                        return Err(format!("непонятный символ «{}»", String::from_utf8_lossy(&src[i..(i + 1).min(src.len())])));
                    }
                    let word = &src[start..i];
                    let number = word[0] == b'-' || word[0].is_ascii_digit();
                    if !number && !matches!(word, b"true" | b"false" | b"null") {
                        return Err(format!("непонятное слово «{}»", String::from_utf8_lossy(&word[..word.len().min(20)])));
                    }
                    self.put(word);
                    seen_value = true;
                }
            }
        }
        if !stack.is_empty() {
            return Err("JSON оборван: не все скобки закрыты".into());
        }
        if !seen_value {
            return Err("пустой ответ".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> ResponseBody {
        ResponseBody::Memory(Bytes::from(text.to_string()))
    }

    fn lines(v: &TextView) -> Vec<String> {
        (0..v.line_count()).map(|i| String::from_utf8_lossy(v.line(i)).into_owned()).collect()
    }

    fn pretty(text: &str) -> Result<TextView, String> {
        TextView::pretty_json(&body(text), 1 << 20, std::env::temp_dir()).unwrap()
    }

    #[test]
    fn pretty_prints_like_jq_with_folds() {
        let v = pretty(r#" {"a":[1, 2,{}], "s":"x\"}{", "e":[ ],"n":{"t":true}} "#).unwrap();
        assert_eq!(
            lines(&v),
            [
                "{", "  \"a\": [", "    1,", "    2,", "    {}", "  ],", "  \"s\": \"x\\\"}{\",", "  \"e\": [],", "  \"n\": {",
                "    \"t\": true", "  }", "}"
            ]
        );
        assert_eq!(v.fold_end(0), Some(11));
        assert_eq!(v.fold_end(1), Some(5));
        assert_eq!(v.fold_end(8), Some(10));
        assert_eq!(v.fold_end(2), None);
        assert_eq!(v.top_folds(10), Some(vec![(1, 5), (8, 10)]));
        assert_eq!(v.top_folds(1), None);
    }

    #[test]
    fn rejects_what_is_not_json() {
        for bad in ["<html></html>", "{\"a\": 1", "[1]]", "{\"a\": tru}", "\"open", "", "{} x", "{\"a\":[1}"] {
            assert!(pretty(bad).is_err(), "{bad}");
        }
        assert!(pretty("42").is_ok());
        assert!(pretty("\u{feff}[\"кот\"]").is_ok());
    }

    #[test]
    fn long_lines_are_cut_on_character_boundaries() {
        let text = "ж".repeat(5000) + "\nend";
        let v = TextView::raw(&body(&text)).unwrap();
        assert_eq!(v.line_count(), 4);
        assert_eq!(v.line(0).len(), MAX_LINE);
        assert!(std::str::from_utf8(v.line(1)).is_ok());
        assert_eq!(v.line(3), b"end");
        let total: usize = (0..v.line_count()).map(|i| v.line(i).len()).sum();
        assert_eq!(total, 10_000 + 3);
    }

    #[test]
    fn index_finds_lines_past_checkpoints() {
        let text: String = (0..5000).map(|i| format!("line {i}\r\n")).collect();
        let v = TextView::raw(&body(&text)).unwrap();
        assert_eq!(v.line_count(), 5000);
        assert_eq!(v.line(4321), b"line 4321");
        assert_eq!(v.line(5000), b"");
    }

    #[test]
    fn search_counts_and_lists_lines() {
        let v = pretty(r#"{"city":"Москва","to":"МОСКВА","x":"москва москва"}"#).unwrap();
        let exact = Matcher::new("москва", true).unwrap();
        assert_eq!(v.search(&exact), SearchResult { total: 2, lines: vec![3] });
        let folded = Matcher::new("москва", false).unwrap();
        assert_eq!(v.search(&folded), SearchResult { total: 4, lines: vec![1, 2, 3] });
        let line = v.line(3);
        assert_eq!(folded.ranges(line).len(), 2);
        assert!(Matcher::new("", false).is_none());
    }

    #[test]
    fn big_pretty_view_goes_to_a_file() {
        let row = r#"{"id":1,"name":"Котик","tags":["a","b"]}"#;
        let text = format!("[{}]", vec![row; 20_000].join(","));
        let dir = tempfile::tempdir().unwrap();
        let v = TextView::pretty_json(&body(&text), 64 << 10, dir.path().to_path_buf()).unwrap().unwrap();
        assert!(matches!(v.data, Data::Mapped(_)));
        assert_eq!(v.line_count(), 2 + 20_000 * 8);
        assert_eq!(v.line(1), b"  {");
        assert_eq!(v.fold_end(1), Some(8));
        assert_eq!(v.line(v.line_count() - 1), b"]");
    }
}
