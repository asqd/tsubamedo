//! Scripts before and after a request, in JavaScript on QuickJS inside the core, so they run
//! the same in the app and in the CLI.
//!
//! The script before sees the request as written, with `{{variables}}`, and may change it or
//! set variables that the same request then uses. The script after sees what went out and what
//! came back, sets variables for other requests and adds tests:
//!
//! ```js
//! // before
//! vars.set("ts", Date.now());
//! request.headers.set("X-Sign", crypto.hmac("sha256", vars.get("secret"), vars.render(request.body)));
//!
//! // after
//! const order = response.json();
//! vars.set("order_id", order.id);
//! test("заказ оплачен", () => expect(order.status).toBe("paid"));
//! ```
//!
//! The sandbox has no files, network or timers: only the language, `vars`, `request`,
//! `response`, `test`/`expect`, `console`, `crypto` and `btoa`/`atob`. A script gets
//! [`TIME_LIMIT`] and [`MEMORY_LIMIT`].
//!
//! Scripts imported from Postman and Insomnia find their `pm`, `insomnia`, `postman`,
//! `tests[…]` and `CryptoJS` there too, as far as such scripts usually go (`compat.js`).

use std::io::Read;
use std::time::{Duration, Instant};

use base64::Engine;
use rquickjs::context::EvalOptions;
use rquickjs::{CaughtError, Context, Ctx, Exception, Function, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::http::Response;
use crate::model::{Body, Pair, Request};
use crate::prepare::Prepared;
use crate::template::Vars;

pub const TIME_LIMIT: Duration = Duration::from_secs(5);
pub const MEMORY_LIMIT: usize = 256 << 20;
/// `response.text()` beyond this throws: a script has no business with 300 MB.
const BODY_LIMIT: u64 = 16 << 20;

const PRELUDE: &str = include_str!("prelude.js");
/// `pm`, `postman`, `insomnia` over the prelude, for scripts imported from those tools.
const COMPAT: &str = include_str!("compat.js");
/// The name errors give the user's script, to tell its lines from the prelude's.
const FILENAME: &str = "script";

/// The request as a script sees and changes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScriptRequest {
    method: String,
    url: String,
    headers: Vec<Pair>,
    query: Vec<Pair>,
    /// The text of a JSON or text body; other bodies are not visible.
    body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub name: String,
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Deserialize)]
struct Output {
    vars: serde_json::Map<String, serde_json::Value>,
    tests: Vec<TestResult>,
    logs: Vec<String>,
    request: ScriptRequest,
}

/// What a script did; `error` is set when it threw or ran out of time, and what it did up to
/// then is kept.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Variables it set, in order.
    pub vars: Vec<(String, String)>,
    pub tests: Vec<TestResult>,
    pub logs: Vec<String>,
    pub error: Option<String>,
}

/// Runs the script before a request; returns the request as the script left it.
pub fn before(script: &str, request: &Request, vars: &Vars) -> (Request, Outcome) {
    let body = match &request.body {
        Body::Json { content } | Body::Text { content, .. } => Some(content.clone()),
        _ => None,
    };
    let input = ScriptRequest {
        method: request.method.clone(),
        url: request.url.clone(),
        headers: request.headers.clone(),
        query: request.query.clone(),
        body,
    };
    let (changed, outcome) = run(script, json!({ "vars": vars, "request": input, "response": null }));
    let mut request = request.clone();
    if let Some(r) = changed {
        request.method = r.method;
        request.url = r.url;
        request.headers = r.headers;
        request.query = r.query;
        if let (Body::Json { content } | Body::Text { content, .. }, Some(text)) = (&mut request.body, r.body) {
            *content = text;
        }
    }
    (request, outcome)
}

/// Runs the script after a response.
pub fn after(script: &str, sent: &Prepared, response: &Response, vars: &Vars) -> Outcome {
    let request = ScriptRequest {
        method: sent.method.clone(),
        url: sent.url.to_string(),
        headers: sent.headers.iter().map(|(n, v)| Pair::new(n, v)).collect(),
        query: sent.url.query_pairs().map(|(n, v)| Pair::new(n, v)).collect(),
        body: Some(String::from_utf8_lossy(&sent.body).into_owned()),
    };
    let (body, body_error) = if response.body.len() > BODY_LIMIT {
        (None, Some(format!("тело больше {} МБ — скрипту его не отдать", BODY_LIMIT >> 20)))
    } else {
        let mut bytes = Vec::new();
        match response.body.reader().and_then(|mut r| r.read_to_end(&mut bytes)) {
            Ok(_) => (Some(String::from_utf8_lossy(&bytes).into_owned()), None),
            Err(e) => (None, Some(e.to_string())),
        }
    };
    let response = json!({
        "status": response.status,
        "reason": response.reason,
        "time": response.timings.total.as_millis() as u64,
        "size": response.body.len(),
        "headers": response.headers.iter().map(|(n, v)| Pair::new(n, v)).collect::<Vec<_>>(),
        "body": body,
        "bodyError": body_error,
    });
    run(script, json!({ "vars": vars, "request": request, "response": response })).1
}

/// Where a script keeps variables: ours, and the scopes of Postman and Insomnia, which are
/// all the same set here.
const SCOPE: &str = r"(?:vars|(?:pm|insomnia)\.(?:environment|collectionVariables|globals|variables|baseEnvironment))";

/// Names a script reads with `vars.get("…")` or `vars.has("…")`, for ordering requests.
pub fn reads(script: &str) -> Vec<String> {
    let mut names = names(script, &format!(r#"{SCOPE}\.(?:get|has)\(\s*["'`]([^"'`]+)["'`]\s*\)"#));
    names.extend(self::names(script, r#"postman\.get(?:Environment|Global)Variable\(\s*["'`]([^"'`]+)["'`]\s*\)"#));
    names
}

/// Names a script sets with `vars.set("…", …)`: the request gives those to others.
pub fn sets(script: &str) -> Vec<String> {
    let mut names = names(script, &format!(r#"{SCOPE}\.set\(\s*["'`]([^"'`]+)["'`]\s*,"#));
    names.extend(self::names(script, r#"postman\.set(?:Environment|Global)Variable\(\s*["'`]([^"'`]+)["'`]\s*,"#));
    names
}

fn names(script: &str, pattern: &str) -> Vec<String> {
    match regex::Regex::new(pattern) {
        Ok(re) => re.captures_iter(script).map(|c| c[1].to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

fn run(script: &str, input: serde_json::Value) -> (Option<ScriptRequest>, Outcome) {
    let failed = |error: String| (None, Outcome { error: Some(error), ..Outcome::default() });
    let Ok(runtime) = Runtime::new() else { return failed("не удалось запустить QuickJS".into()) };
    runtime.set_memory_limit(MEMORY_LIMIT);
    runtime.set_max_stack_size(1 << 20);
    let deadline = Instant::now() + TIME_LIMIT;
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline)));
    let Ok(context) = Context::full(&runtime) else { return failed("не удалось запустить QuickJS".into()) };

    context.with(|ctx| {
        if let Err(e) = install(&ctx, &input.to_string()) {
            return failed(format!("окружение скрипта: {}", describe(&ctx, e)));
        }
        // Not strict, like the scripts of other HTTP clients: `x = 1` just works.
        let mut options = EvalOptions::default();
        options.strict = false;
        options.filename = Some(FILENAME.into());
        let result = ctx.eval_with_options::<(), _>(script, options);
        // Promises the script started settle now; there are no timers to wait for.
        while result.is_ok() && ctx.execute_pending_job() {}
        let error = result.err().map(|e| {
            if Instant::now() > deadline {
                format!("скрипт работал дольше {} с и был остановлен", TIME_LIMIT.as_secs())
            } else {
                describe(&ctx, e)
            }
        });
        // What it managed before an error is still worth showing.
        let output: Option<Output> = ctx.eval::<String, _>("__output()").ok().and_then(|s| serde_json::from_str(&s).ok());
        let Some(output) = output else { return failed(error.unwrap_or_else(|| "скрипт сломал своё окружение".into())) };
        let vars = output
            .vars
            .into_iter()
            .map(|(k, v)| (k, v.as_str().map_or_else(|| v.to_string(), str::to_string)))
            .collect();
        (Some(output.request), Outcome { vars, tests: output.tests, logs: output.logs, error })
    })
}

/// The input, the natives the prelude wraps, then the prelude.
fn install(ctx: &Ctx<'_>, input: &str) -> rquickjs::Result<()> {
    let g = ctx.globals();
    g.set("__input", input)?;
    g.set("__hash", Function::new(ctx.clone(), |ctx: Ctx<'_>, alg: String, text: String, encoding: String| {
        let alg = match alg.as_str() {
            "sha1" => &ring::digest::SHA1_FOR_LEGACY_USE_ONLY,
            "sha256" => &ring::digest::SHA256,
            "sha512" => &ring::digest::SHA512,
            _ => return Err(Exception::throw_message(&ctx, &format!("неизвестный алгоритм {alg}"))),
        };
        encode(&ctx, ring::digest::digest(alg, text.as_bytes()).as_ref(), &encoding)
    })?)?;
    g.set("__hmac", Function::new(ctx.clone(), |ctx: Ctx<'_>, alg: String, key: String, text: String, encoding: String| {
        let alg = match alg.to_ascii_lowercase().as_str() {
            "sha1" => ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            "sha256" => ring::hmac::HMAC_SHA256,
            "sha384" => ring::hmac::HMAC_SHA384,
            "sha512" => ring::hmac::HMAC_SHA512,
            _ => return Err(Exception::throw_message(&ctx, &format!("hmac: неизвестный алгоритм {alg}, есть sha1, sha256, sha384, sha512"))),
        };
        let tag = ring::hmac::sign(&ring::hmac::Key::new(alg, key.as_bytes()), text.as_bytes());
        encode(&ctx, tag.as_ref(), &encoding)
    })?)?;
    g.set("__b64", Function::new(ctx.clone(), |ctx: Ctx<'_>, text: String, decode: bool| {
        let b64 = base64::engine::general_purpose::STANDARD;
        if !decode {
            return Ok(b64.encode(text.as_bytes()));
        }
        let bytes = b64.decode(text.trim()).map_err(|e| Exception::throw_message(&ctx, &format!("atob: {e}")))?;
        String::from_utf8(bytes).map_err(|_| Exception::throw_message(&ctx, "atob: внутри не текст UTF-8"))
    })?)?;
    g.set("__uuid", Function::new(ctx.clone(), || uuid::Uuid::new_v4().to_string())?)?;
    for (code, name) in [(PRELUDE, "prelude"), (COMPAT, "compat")] {
        let mut options = EvalOptions::default();
        options.filename = Some(name.into());
        ctx.eval_with_options::<(), _>(code, options)?;
    }
    Ok(())
}

fn encode(ctx: &Ctx<'_>, bytes: &[u8], encoding: &str) -> rquickjs::Result<String> {
    match encoding {
        "hex" => Ok(bytes.iter().map(|b| format!("{b:02x}")).collect()),
        "base64" => Ok(base64::engine::general_purpose::STANDARD.encode(bytes)),
        other => Err(Exception::throw_message(ctx, &format!("кодировка {other}: есть hex и base64"))),
    }
}

/// The exception's message with the script line it came from.
fn describe(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    match CaughtError::from_error(ctx, error) {
        CaughtError::Exception(e) => {
            let message = e.message().unwrap_or_else(|| "ошибка".into());
            let line = e.stack().and_then(|stack| {
                stack.lines().find(|l| l.contains(&format!("{FILENAME}:"))).and_then(|l| {
                    let mut parts = l.rsplit(':');
                    parts.next()?;
                    parts.next()?.parse::<u32>().ok()
                })
            });
            match line {
                Some(n) => format!("{message} (строка {n})"),
                None => message,
            }
        }
        CaughtError::Value(v) => format!("выброшено {v:?}"),
        CaughtError::Error(e) => e.to_string(),
    }
}

#[cfg(test)]
mod tests;
