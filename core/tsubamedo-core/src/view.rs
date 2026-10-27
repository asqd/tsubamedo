//! Text views of a response body for the viewer, built for bodies of hundreds of megabytes.
//!
//! The interface never gets the whole body: it asks for the lines it shows. A view is the
//! body (raw) or its pretty-printed JSON (written to temporary files the same way big bodies
//! are), plus a line index of 2 bytes per line and the fold ranges of JSON objects and arrays.
//! Lines longer than [`MAX_LINE`] bytes are cut into several, so minified JSON on one line
//! still scrolls.
//!
//! Big JSON is formatted on all cores: the body is cut into parts right after commas outside
//! strings, each part is formatted into its own segment, and the segments are read as one text.

use std::io::{self, Write};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;

use crate::http::ResponseBody;
use crate::http::body::Sink;

pub const MAX_LINE: usize = 4096;
const CHECKPOINT: usize = 1024;
/// Search stops counting here; nobody reads further, and it keeps a pathological query fast.
pub const MAX_MATCHES: usize = 1_000_000;
const MAX_MATCH_LINES: usize = 100_000;
/// Deeper JSON is not indented: every line would carry kilobytes of spaces.
const MAX_DEPTH: usize = 1000;
/// Bodies are cut into parts of at least this size for formatting on several cores.
const MIN_PART: usize = 1 << 20;

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

/// A piece of the text that starts and ends on a line boundary.
struct Segment {
    data: Data,
    index: LineIndex,
    /// The view's line this segment starts with.
    first: usize,
    // Keeps the temporary file alive as long as the view is.
    _body: ResponseBody,
}

impl Segment {
    fn new(body: ResponseBody, index: LineIndex, first: usize) -> io::Result<Self> {
        Ok(Segment { data: Data::of(&body)?, index, first, _body: body })
    }

    fn line_count(&self) -> usize {
        self.index.lens.len()
    }
}

pub struct TextView {
    segments: Vec<Segment>,
    lines: usize,
    len: u64,
    /// `(open, close)` lines of every multi-line object or array, sorted by `open`.
    folds: Vec<(u32, u32)>,
    pub pretty: bool,
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
        let segment = Segment { data, index, first: 0, _body: body.clone() };
        Ok(TextView { lines: segment.line_count(), len: segment.data.bytes().len() as u64, segments: vec![segment], folds: Vec::new(), pretty: false })
    }

    /// The body as indented JSON; `Ok(Err(reason))` when it is not JSON.
    pub fn pretty_json(body: &ResponseBody, spill_threshold: usize, spill_dir: PathBuf) -> io::Result<Result<Self, String>> {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let parts = (body.len() as usize / MIN_PART).clamp(1, threads * 4);
        Self::pretty_in_parts(body, parts, spill_threshold, spill_dir)
    }

    fn pretty_in_parts(body: &ResponseBody, parts: usize, spill_threshold: usize, spill_dir: PathBuf) -> io::Result<Result<Self, String>> {
        let source = Data::of(body)?;
        let src = source.bytes();
        let src = src.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(src);
        let bounds = boundaries(src, parts);
        let n = bounds.len() - 1;
        let range = |k: usize| bounds[k]..bounds[k + 1];

        // What is open where each part starts, from the brackets each part closes and opens.
        let mut starts = vec![Vec::new()];
        if n > 1 {
            let mut stack: Vec<u8> = Vec::new();
            for (closes, opens) in par_map(n - 1, |k| brackets(&src[range(k)])) {
                for close in closes {
                    match stack.pop() {
                        Some(want) if want == close => {}
                        Some(_) => return Ok(Err("скобки не совпадают".into())),
                        None => return Ok(Err("лишняя закрывающая скобка".into())),
                    }
                }
                stack.extend(opens);
                if stack.len() > MAX_DEPTH {
                    return Ok(Err(format!("вложенность глубже {MAX_DEPTH} уровней")));
                }
                starts.push(stack.clone());
            }
        }

        let threshold = if src.len() > spill_threshold { spill_threshold / n } else { spill_threshold };
        let formatted = par_map(n, |k| Part::format(src, range(k), &starts[k], k + 1 == n, threshold, spill_dir.clone()));

        // Glue the parts: number their lines in order and pair up the brackets that one part
        // opens and another one closes.
        let mut segments = Vec::with_capacity(n);
        let mut folds = Vec::new();
        let mut across = Vec::new();
        let mut open: Vec<u32> = Vec::new();
        let mut lines = 0;
        for (k, part) in formatted.into_iter().enumerate() {
            let part = match part? {
                Ok(part) => part,
                Err(reason) => return Ok(Err(reason)),
            };
            if starts.get(k + 1).is_some_and(|next| *next != part.end) {
                return Ok(Err("скобки не совпадают".into()));
            }
            let first = lines as u32;
            for close in part.closed {
                let Some(line) = open.pop() else { return Ok(Err("лишняя закрывающая скобка".into())) };
                across.push((line, first + close));
            }
            open.extend(part.open.iter().map(|&line| first + line));
            folds.extend(part.folds.iter().map(|&(a, b)| (first + a, first + b)));
            let segment = Segment::new(part.body, part.index, lines)?;
            lines += segment.line_count();
            segments.push(segment);
        }
        // Each part's folds are sorted and the parts are in order; those across parts slot in.
        if !across.is_empty() {
            across.sort_unstable();
            let mut merged = Vec::with_capacity(folds.len() + across.len());
            let mut across = across.into_iter().peekable();
            for fold in folds {
                while let Some(&a) = across.peek()
                    && a < fold
                {
                    merged.push(a);
                    across.next();
                }
                merged.push(fold);
            }
            merged.extend(across);
            folds = merged;
        }
        folds.shrink_to_fit();
        let len = segments.iter().map(|s| s.data.bytes().len() as u64).sum();
        Ok(Ok(TextView { segments, lines, len, folds, pretty: true }))
    }

    pub fn line_count(&self) -> usize {
        self.lines
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The line without its line break.
    pub fn line(&self, n: usize) -> &[u8] {
        if n >= self.line_count() {
            return &[];
        }
        let s = &self.segments[self.segments.partition_point(|s| s.first <= n) - 1];
        let local = n - s.first;
        let start = s.index.start(local) as usize;
        let mut line = &s.data.bytes()[start..start + s.index.lens[local] as usize];
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
        for s in &self.segments {
            let mut line = 0usize;
            let mut line_end = s.index.lens.first().map_or(0, |&l| l as u64);
            for start in matcher.find_iter(s.data.bytes()) {
                let start = start as u64;
                while start >= line_end && line + 1 < s.line_count() {
                    line += 1;
                    line_end += s.index.lens[line] as u64;
                }
                result.total += 1;
                let n = (s.first + line) as u32;
                if result.lines.last() != Some(&n) && result.lines.len() < MAX_MATCH_LINES {
                    result.lines.push(n);
                }
                if result.total >= MAX_MATCHES {
                    return result;
                }
            }
        }
        result
    }

    /// The whole view, for copying or saving the pretty text.
    pub fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        self.segments.iter().try_for_each(|s| out.write_all(s.data.bytes()))
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

/// `f(0..n)` on all cores, in order.
fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(1, |t| t.get()).min(n);
    if threads <= 1 {
        return (0..n).map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let mut out: Vec<Option<T>> = (0..n).map(|_| None).collect();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= n {
                            return done;
                        }
                        done.push((i, f(i)));
                    }
                })
            })
            .collect();
        for worker in workers {
            let done = worker.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic));
            for (i, value) in done {
                out[i] = Some(value);
            }
        }
    });
    out.into_iter().map(|v| v.expect("every part is done")).collect()
}

/// Where the string whose text starts at `from` ends (past its closing quote).
fn string_end(src: &[u8], mut from: usize) -> Option<usize> {
    loop {
        let pos = memchr::memchr2(b'"', b'\\', src.get(from..)?)?;
        if src[from + pos] == b'\\' {
            from += pos + 2;
        } else {
            return Some(from + pos + 1);
        }
    }
}

/// Cuts the JSON into about `parts` parts, each starting right after a comma outside strings,
/// where formatting can start knowing only which brackets are open. Returns the cut points
/// with 0 and the end.
fn boundaries(src: &[u8], parts: usize) -> Vec<usize> {
    let mut bounds = vec![0];
    let parts = parts.min(src.len());
    if parts > 1 {
        // Rough points, none right after a backslash, so a quote there is never escaped.
        let mut rough: Vec<usize> = (0..=parts).map(|k| src.len() / parts * k).collect();
        rough[parts] = src.len();
        for k in 1..parts {
            rough[k] = rough[k].max(rough[k - 1]);
            while rough[k] < src.len() && src[rough[k] - 1] == b'\\' {
                rough[k] += 1;
            }
        }
        // Whether each rough point is inside a string: count unescaped quotes before it. In
        // JSON a backslash only appears inside strings, so this does not depend on where
        // the part starts.
        let odd = par_map(parts - 1, |k| {
            let from = rough[k];
            memchr::memchr_iter(b'"', &src[from..rough[k + 1]]).fold(false, |odd, q| {
                let slashes = src[..from + q].iter().rev().take_while(|&&b| b == b'\\').count();
                odd ^ slashes.is_multiple_of(2)
            })
        });
        let mut in_string = false;
        for (k, odd) in odd.into_iter().enumerate() {
            in_string ^= odd;
            let mut i = rough[k + 1];
            if in_string {
                let Some(end) = string_end(src, i) else { break };
                i = end;
            }
            // The next comma outside strings.
            let cut = loop {
                let Some(pos) = memchr::memchr2(b'"', b',', &src[i..]) else { break None };
                if src[i + pos] == b',' {
                    break Some(i + pos + 1);
                }
                match string_end(src, i + pos + 1) {
                    Some(end) => i = end,
                    None => break None,
                }
            };
            match cut {
                Some(cut) if cut > *bounds.last().unwrap() && cut < src.len() => bounds.push(cut),
                Some(_) => {}
                None => break,
            }
        }
    }
    bounds.push(src.len());
    bounds
}

/// The brackets a part closes that earlier parts opened, in order, and those it leaves open
/// (as their closing brackets). Only the brackets: the part is checked when it is formatted.
fn brackets(src: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (mut closes, mut opens) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < src.len() {
        match src[i] {
            b'"' => match string_end(src, i + 1) {
                Some(end) => {
                    i = end;
                    continue;
                }
                None => break,
            },
            b'{' => opens.push(b'}'),
            b'[' => opens.push(b']'),
            close @ (b'}' | b']') => match opens.pop() {
                Some(_) => {}
                None => closes.push(close),
            },
            _ => {}
        }
        i += 1;
    }
    (closes, opens)
}

/// One part of the JSON, formatted.
struct Part {
    body: ResponseBody,
    index: LineIndex,
    /// Folds that open and close in this part, in its own line numbers, sorted.
    folds: Vec<(u32, u32)>,
    /// Lines that close what earlier parts opened, in order.
    closed: Vec<u32>,
    /// Lines that open what later parts close, outermost first.
    open: Vec<u32>,
    /// What is open after the part (closing brackets, outermost first).
    end: Vec<u8>,
}

impl Part {
    fn format(src: &[u8], range: Range<usize>, start: &[u8], last: bool, threshold: usize, dir: PathBuf) -> io::Result<Result<Self, String>> {
        let first = range.start == 0;
        let mut out = Formatter {
            sink: Sink::new(threshold, dir),
            index: LineIndex::default(),
            folds: Vec::new(),
            closed: Vec::new(),
            error: None,
            // A part starts after a comma, on a new line.
            indent: (!first).then_some(start.len()),
        };
        let (stack, inherited) = match out.format(src, range, start, first, last) {
            Ok(done) => done,
            Err(reason) => return Ok(Err(reason)),
        };
        if let Some(e) = out.error {
            return Err(e);
        }
        out.index.finish();
        out.folds.sort_unstable();
        Ok(Ok(Part {
            body: out.sink.finish()?,
            index: out.index,
            folds: out.folds,
            closed: out.closed,
            open: stack[inherited..].iter().map(|&(_, line)| line).collect(),
            end: stack.iter().map(|&(close, _)| close).collect(),
        }))
    }
}

/// A streaming JSON pretty-printer: two-space indent, like `jq`. It checks brackets, strings
/// and scalars, not every rule of JSON; anything it does not understand means "not JSON".
struct Formatter {
    sink: Sink,
    index: LineIndex,
    folds: Vec<(u32, u32)>,
    closed: Vec<u32>,
    error: Option<io::Error>,
    /// Indentation owed to the next token. Written lazily, so a part ends right after its
    /// last line break and the next part starts with the indentation.
    indent: Option<usize>,
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

    fn token(&mut self, bytes: &[u8]) {
        const SPACES: &[u8] = &[b' '; 128];
        if let Some(depth) = self.indent.take() {
            let mut left = depth * 2;
            while left > 0 {
                let n = left.min(SPACES.len());
                self.put(&SPACES[..n]);
                left -= n;
            }
        }
        self.put(bytes);
    }

    fn newline(&mut self, depth: usize) {
        self.put(b"\n");
        self.indent = Some(depth);
    }

    /// Formats `src[range]`, which starts outside strings with the `start` brackets open.
    /// Returns what is open at the end with the lines it opened on, and how many of those
    /// came from `start`.
    fn format(&mut self, src: &[u8], range: Range<usize>, start: &[u8], first: bool, last: bool) -> Result<(Vec<(u8, u32)>, usize), String> {
        let ws = |b: u8| matches!(b, b' ' | b'\t' | b'\r' | b'\n');
        let end = range.end;
        let src = &src[..end];
        let mut stack: Vec<(u8, u32)> = start.iter().map(|&close| (close, 0)).collect();
        let mut inherited = stack.len();
        let mut seen_value = !first;
        let mut i = range.start;
        while i < end {
            let b = src[i];
            if stack.is_empty() && seen_value && !ws(b) {
                return Err("после JSON есть ещё текст".into());
            }
            match b {
                b' ' | b'\t' | b'\r' | b'\n' => i += 1,
                b'"' => {
                    let Some(j) = string_end(src, i + 1) else { return Err("строка не закрыта".into()) };
                    self.token(&src[i..j]);
                    i = j;
                    seen_value = true;
                }
                b'{' | b'[' => {
                    let close = if b == b'{' { b'}' } else { b']' };
                    let mut k = i + 1;
                    while k < end && ws(src[k]) {
                        k += 1;
                    }
                    if src.get(k) == Some(&close) {
                        self.token(&[b, close]);
                        i = k + 1;
                        seen_value = true;
                    } else {
                        if stack.len() == MAX_DEPTH {
                            return Err(format!("вложенность глубже {MAX_DEPTH} уровней"));
                        }
                        self.token(&[b]);
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
                    let line = self.index.line();
                    if stack.len() < inherited {
                        inherited = stack.len();
                        self.closed.push(line);
                    } else {
                        self.folds.push((open, line));
                    }
                    self.token(&[b]);
                    i += 1;
                    seen_value = true;
                }
                b',' if !stack.is_empty() => {
                    self.token(b",");
                    self.newline(stack.len());
                    i += 1;
                }
                b':' if stack.last().is_some_and(|&(c, _)| c == b'}') => {
                    self.token(b": ");
                    i += 1;
                }
                _ => {
                    let start = i;
                    while i < end && matches!(src[i], b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'+' | b'-' | b'.') {
                        i += 1;
                    }
                    if i == start {
                        return Err(format!("непонятный символ «{}»", String::from_utf8_lossy(&src[i..(i + 1).min(end)])));
                    }
                    let word = &src[start..i];
                    let number = word[0] == b'-' || word[0].is_ascii_digit();
                    if !number && !matches!(word, b"true" | b"false" | b"null") {
                        return Err(format!("непонятное слово «{}»", String::from_utf8_lossy(&word[..word.len().min(20)])));
                    }
                    self.token(word);
                    seen_value = true;
                }
            }
        }
        if last && !stack.is_empty() {
            return Err("JSON оборван: не все скобки закрыты".into());
        }
        if !seen_value {
            return Err("пустой ответ".into());
        }
        Ok((stack, inherited))
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

    fn pretty_in(text: &str, parts: usize) -> Result<TextView, String> {
        TextView::pretty_in_parts(&body(text), parts, 1 << 20, std::env::temp_dir()).unwrap()
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
        let deep = "[1,".repeat(MAX_DEPTH + 1) + &"]".repeat(MAX_DEPTH + 1);
        assert!(pretty(&deep).is_err());
        assert!(pretty_in(&deep, 8).is_err());
    }

    #[test]
    fn parts_give_the_same_view_as_one_pass() {
        // Commas, brackets and escaped quotes inside strings, folds across parts, empty
        // containers, deep nesting and long strings: wherever the rough cuts fall.
        let row = |i: usize| {
            format!(
                r#"{{"id":{i},"s":"a, b [c] {{d}} \"e,\" \\","t":[ ],"o":{{ }},"deep":[[[{{"x":[1,2,{{"y":"{}"}}]}}]]],"ru":"Москва, Сочи"}}"#,
                "ж,\\\"".repeat(i % 7 * 300)
            )
        };
        let text = format!(r#"{{"list":[{}],"tail":{{"k":[true,false,null]}}}}"#, (0..60).map(row).collect::<Vec<_>>().join(" ,\n "));
        let one = pretty_in(&text, 1).unwrap();
        for parts in [2, 3, 7, 50, 400, 5000] {
            let many = pretty_in(&text, parts).unwrap();
            assert!(many.segments.len() > 1 || parts == 1);
            assert_eq!(lines(&many), lines(&one), "{parts} parts");
            assert_eq!(many.folds, one.folds, "{parts} parts");
            let mut joined = Vec::new();
            many.write_to(&mut joined).unwrap();
            assert_eq!(joined.len() as u64, one.len());
            let m = Matcher::new("сочи", false).unwrap();
            assert_eq!(many.search(&m), one.search(&m));
        }
        for bad in [&text[..text.len() - 1], &text.replacen("]}", "}}", 1), &format!("{text},1")] {
            assert!(pretty_in(bad, 1).is_err());
            for parts in [2, 7, 50] {
                assert!(pretty_in(bad, parts).is_err(), "{parts} parts");
            }
        }
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
    fn big_pretty_view_goes_to_files() {
        let row = r#"{"id":1,"name":"Котик","tags":["a","b"]}"#;
        let text = format!("[{}]", vec![row; 20_000].join(","));
        let dir = tempfile::tempdir().unwrap();
        let v = TextView::pretty_in_parts(&body(&text), 4, 64 << 10, dir.path().to_path_buf()).unwrap().unwrap();
        assert_eq!(v.segments.len(), 4);
        assert!(v.segments.iter().all(|s| matches!(s.data, Data::Mapped(_))));
        assert_eq!(v.line_count(), 2 + 20_000 * 8);
        assert_eq!(v.line(1), b"  {");
        assert_eq!(v.fold_end(1), Some(8));
        assert_eq!(v.fold_end(0), Some(v.line_count() as u32 - 1));
        assert_eq!(v.line(v.line_count() - 1), b"]");
        drop(v);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
