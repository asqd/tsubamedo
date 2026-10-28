//! Requests that need each other. A request keeps values from its response (`[capture]
//! token = "$.token"`), and others use them as plain `{{token}}`. When a request is sent and
//! such a variable is not set yet, the request that captures it goes first; a collection run
//! orders requests so that those go first anyway.
//!
//! Captured values sit above the collection's files and below the caller's overrides
//! (`--var` in the CLI): a fresh token beats the placeholder in `dev.local.toml`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::checks::{self, Answer, CheckResult, Captured};
use crate::collection::{Collection, Entry};
use crate::http::{self, CertSpec, ClientCert, Response, SendOptions};
use crate::template::render;
use crate::oauth::{self, Mode, SharedTokens, Tokens};
use crate::prepare::{Prepared, prepare, scripts_variables, variables};
use crate::{digest, scripts, sigv4};
use crate::{Error, Request, Result, Vars};

/// How deep requests may pull in other requests.
const MAX_CHAIN: usize = 8;
/// The pause before a failed request is sent again, times the attempt.
const RETRY_PAUSE: Duration = Duration::from_millis(500);

/// A request that captures a variable.
#[derive(Debug, Clone)]
pub struct Source {
    pub id: String,
    pub request: Request,
    pub dir: PathBuf,
}

/// Which request captures each variable (in `[capture]` or with `vars.set` in its script after
/// the response); the first in the collection's order wins.
pub fn producers(collection: &Collection) -> BTreeMap<String, Source> {
    let mut out = BTreeMap::new();
    for entry in &collection.requests {
        let captured = entry.request.captures.iter().filter(|c| c.enabled).map(|c| c.name.trim().to_string());
        for name in captured.chain(scripts::sets(&entry.request.scripts.after)) {
            out.entry(name).or_insert_with(|| Source {
                id: entry.id.clone(),
                request: entry.request.clone(),
                dir: entry.dir().to_path_buf(),
            });
        }
    }
    out
}

/// One request sent: what went out, what came back, the checks and the captured values.
pub struct Exchange {
    /// The collection request, `None` for a scratch one.
    pub id: Option<String>,
    pub request: Request,
    pub sent: Option<Prepared>,
    pub result: Result<Response>,
    pub checks: Vec<CheckResult>,
    pub captures: Vec<Captured>,
    /// What the scripts wrote with `console.log`.
    pub logs: Vec<String>,
}

impl Exchange {
    /// A response came, every check holds and every value was found.
    pub fn passed(&self) -> bool {
        self.result.is_ok() && self.checks.iter().all(|c| c.ok) && self.captures.iter().all(|c| c.error.is_none())
    }

    pub fn failures(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Err(e) = &self.result {
            out.push(e.to_string());
        }
        for c in self.checks.iter().filter(|c| !c.ok) {
            let why = match (&c.error, &c.actual) {
                (Some(e), _) => e.clone(),
                (None, Some(actual)) => format!("пришло {actual}"),
                (None, None) => "не выполнено".into(),
            };
            out.push(format!("{} — {why}", c.check));
        }
        for c in self.captures.iter().filter(|c| c.error.is_some()) {
            out.push(format!("{}: {}", c.name, c.error.as_deref().unwrap_or_default()));
        }
        out
    }
}

/// A request of a collection run and the steps whose values it takes: it starts after them.
#[derive(Debug, Clone)]
pub struct Step {
    pub id: String,
    pub request: Request,
    pub dir: PathBuf,
    pub after: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Requests in flight at once; 1 keeps the collection's order exactly.
    pub parallel: usize,
    /// How many times a failed request is sent again.
    pub retries: u32,
    /// A status of 400 and above fails the request too.
    pub fail_status: bool,
    /// Start nothing new after the first failure.
    pub bail: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions { parallel: 1, retries: 0, fail_status: false, bail: false }
    }
}

/// What a run reports as it goes.
pub enum Event {
    /// A step went out; `attempt` is 2 and more for a retry.
    Started { step: usize, attempt: u32 },
    /// A step is done for good: its exchanges (requests sent first for missing values, then
    /// the step itself) and the attempts it took.
    Finished { step: usize, attempts: u32, exchanges: Vec<Exchange>, passed: bool },
}

#[derive(Clone)]
pub struct Runner {
    base: Vars,
    overrides: Vars,
    /// Values captured from responses so far.
    pub captured: Vars,
    producers: BTreeMap<String, Source>,
    /// Requests that did not give their values; they are not sent again in this run.
    failed: BTreeSet<String>,
    options: SendOptions,
    /// For a fork: the captured values it started with, to tell what it brought.
    forked_from: Vars,
    /// OAuth 2.0 tokens; forks share them.
    tokens: SharedTokens,
}

impl Runner {
    pub fn new(collection: Option<&Collection>, env: Option<&str>, overrides: Vars, captured: Vars, mut options: SendOptions) -> Result<Self> {
        let (base, producers) = match collection {
            Some(c) => (c.vars(env, &Vars::new())?, producers(c)),
            None => (Vars::new(), BTreeMap::new()),
        };
        // The collection's client certificates, after those the caller gave (`--cert`).
        if let Some(c) = collection {
            let specs = c.certificates()?;
            if !specs.is_empty() {
                let mut vars = base.clone();
                vars.extend(overrides.iter().map(|(k, v)| (k.clone(), v.clone())));
                let mut certs = (*options.client_certs).clone();
                certs.extend(specs.iter().map(|s| certificate(s, &vars, &c.root)));
                options.client_certs = Arc::new(certs);
            }
        }
        Ok(Runner {
            base,
            overrides,
            captured,
            producers,
            failed: BTreeSet::new(),
            options,
            forked_from: Vars::new(),
            tokens: Tokens::default().shared(),
        })
    }

    /// Tokens kept from before (the session's), instead of none.
    pub fn with_tokens(mut self, tokens: SharedTokens) -> Self {
        self.tokens = tokens;
        self
    }

    pub fn vars(&self) -> Vars {
        let mut vars = self.base.clone();
        vars.extend(self.captured.iter().chain(&self.overrides).map(|(k, v)| (k.clone(), v.clone())));
        vars
    }

    /// The requests to run, in the collection's order, except that a request comes after
    /// those whose values it uses. Those are run even when `wanted` leaves them out, unless
    /// the caller set the value itself.
    pub fn order<'c>(&self, collection: &'c Collection, wanted: impl Fn(&Entry) -> bool) -> Vec<&'c Entry> {
        let vars = self.vars();
        let mut out: Vec<&Entry> = Vec::new();
        let mut seen = BTreeSet::new();
        fn visit<'c>(runner: &Runner, c: &'c Collection, vars: &Vars, entry: &'c Entry, seen: &mut BTreeSet<String>, out: &mut Vec<&'c Entry>) {
            if !seen.insert(entry.id.clone()) {
                return;
            }
            for name in variables(&entry.request, vars) {
                if runner.overrides.contains_key(&name) {
                    continue;
                }
                if let Some(source) = runner.producers.get(&name)
                    && source.id != entry.id
                    && let Some(dep) = c.find(&source.id)
                {
                    visit(runner, c, vars, dep, seen, out);
                }
            }
            out.push(entry);
        }
        for entry in collection.requests.iter().filter(|e| wanted(e)) {
            visit(self, collection, &vars, entry, &mut seen, &mut out);
        }
        out
    }

    /// The steps of a run: [`Runner::order`] with what each step waits for.
    pub fn plan(&self, collection: &Collection, wanted: impl Fn(&Entry) -> bool) -> Vec<Step> {
        let entries = self.order(collection, wanted);
        self.steps(&entries)
    }

    /// Exactly these requests, in the collection's order: to send again what failed, without
    /// what already went well.
    pub fn plan_only(&self, collection: &Collection, ids: &[String]) -> Vec<Step> {
        let entries = self.order(collection, |e| ids.contains(&e.id));
        let only: Vec<&Entry> = entries.into_iter().filter(|e| ids.contains(&e.id)).collect();
        self.steps(&only)
    }

    fn steps(&self, entries: &[&Entry]) -> Vec<Step> {
        let vars = self.vars();
        let index: HashMap<&str, usize> = entries.iter().enumerate().map(|(i, e)| (e.id.as_str(), i)).collect();
        entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let mut after: Vec<usize> = variables(&e.request, &vars)
                    .iter()
                    .filter(|name| !self.overrides.contains_key(*name))
                    .filter_map(|name| index.get(self.producers.get(name)?.id.as_str()).copied())
                    .filter(|&d| d < i)
                    .collect();
                after.sort_unstable();
                after.dedup();
                Step { id: e.id.clone(), request: e.request.clone(), dir: e.dir().to_path_buf(), after }
            })
            .collect()
    }

    /// Runs the steps: each after those it waits for, up to `parallel` at once, a failed one
    /// again up to `retries` times. Requests running side by side each see the values captured
    /// before they started; what they capture is merged as they finish.
    pub async fn run(&mut self, steps: &[Step], options: &RunOptions, mut on: impl FnMut(Event)) {
        let parallel = options.parallel.max(1);
        let mut started = vec![false; steps.len()];
        let mut done = vec![false; steps.len()];
        let mut attempts = vec![0u32; steps.len()];
        let mut stop = false;
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            // In the steps' order, so one at a time is exactly the collection's order.
            for (i, step) in steps.iter().enumerate() {
                if stop || tasks.len() >= parallel {
                    break;
                }
                if started[i] || !step.after.iter().all(|&d| done[d]) {
                    continue;
                }
                started[i] = true;
                attempts[i] = 1;
                on(Event::Started { step: i, attempt: 1 });
                tasks.spawn(attempt(self.fork(), step.clone(), i, Duration::ZERO));
            }
            let Some(joined) = tasks.join_next().await else { break };
            let Ok((i, fork, exchanges)) = joined else { continue };
            self.join(fork);
            let step = &steps[i];
            let passed = exchanges.last().is_some_and(|ex| {
                ex.passed() && !(options.fail_status && ex.result.as_ref().is_ok_and(|r| r.status >= 400))
            });
            // A request that never went out (a variable missing, a broken script) fails the
            // same way again.
            let went = exchanges.last().is_some_and(|ex| ex.sent.is_some());
            if !passed && went && attempts[i] <= options.retries {
                attempts[i] += 1;
                on(Event::Started { step: i, attempt: attempts[i] });
                tasks.spawn(attempt(self.fork(), step.clone(), i, RETRY_PAUSE * (attempts[i] - 1)));
                continue;
            }
            if !passed {
                // Those that take its values do not send it again each.
                self.failed.insert(step.id.clone());
                stop |= options.bail;
            }
            done[i] = true;
            on(Event::Finished { step: i, attempts: attempts[i], exchanges, passed });
        }
    }

    /// A copy for one request of a parallel run.
    fn fork(&self) -> Runner {
        Runner { forked_from: self.captured.clone(), ..self.clone() }
    }

    /// What a fork captured and learned, without undoing what others captured meanwhile.
    fn join(&mut self, fork: Runner) {
        for (name, value) in fork.captured {
            if fork.forked_from.get(&name) != Some(&value) {
                self.captured.insert(name, value);
            }
        }
        self.failed.extend(fork.failed);
    }

    /// Sends a request. When variables it needs are not set and other requests capture them,
    /// those are sent first; they come before it in the result.
    pub async fn send(&mut self, id: Option<&str>, request: &Request, dir: &Path) -> Vec<Exchange> {
        let mut out = Vec::new();
        let last = self.exchange(id.map(str::to_string), request.clone(), dir.to_path_buf(), 0, &mut out).await;
        out.push(last);
        out
    }

    fn exchange<'a>(
        &'a mut self,
        id: Option<String>,
        request: Request,
        dir: PathBuf,
        depth: usize,
        before: &'a mut Vec<Exchange>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Exchange> + Send + 'a>> {
        Box::pin(async move {
            // The script before may set variables this request uses, so it runs before
            // substitution, and again if requests had to be sent first for missing values.
            let wanted = scripts_variables(&request);
            let mut logs = Vec::new();
            let mut set_by_script = Vec::new();
            let mut tried_producers = false;
            let (scripted, prepared) = loop {
                let scripted = if request.scripts.before.trim().is_empty() {
                    request.clone()
                } else {
                    let (scripted, out) = scripts::before(&request.scripts.before, &request, &self.vars());
                    logs.extend(out.logs);
                    for (name, value) in out.vars {
                        self.captured.insert(name.clone(), value.clone());
                        set_by_script.retain(|c: &Captured| c.name != name);
                        set_by_script.push(Captured { name, value: Some(value), error: None });
                    }
                    if let Some(e) = out.error {
                        let result = Err(Error::Script(format!("скрипт до запроса: {e}")));
                        return Exchange { id, request, sent: None, result, checks: Vec::new(), captures: set_by_script, logs };
                    }
                    scripted
                };
                let prepared = prepare(&scripted, &self.vars(), &dir);
                if tried_producers || depth >= MAX_CHAIN {
                    break (scripted, prepared);
                }
                let vars = self.vars();
                let mut missing = match &prepared {
                    Err(Error::UndefinedVars(names)) => names.clone(),
                    _ => Vec::new(),
                };
                missing.extend(wanted.iter().filter(|n| !vars.contains_key(*n)).cloned());
                tried_producers = true;
                if !self.send_producers(&missing, id.as_deref(), depth, before).await {
                    break (scripted, prepared);
                }
            };
            let mut prepared = match prepared {
                Ok(p) => p,
                Err(e) => return Exchange { id, request, sent: None, result: Err(e), checks: Vec::new(), captures: set_by_script, logs },
            };
            // A header the user wrote themselves wins over the token.
            let oauth = prepared.oauth.clone().filter(|_| prepared.header("authorization").is_none());
            let mut kept_token = false;
            if let Some(o) = &oauth {
                match oauth::token(o, &self.tokens, &self.options, Mode::Cached).await {
                    Ok(got) => {
                        logs.extend(got.note);
                        kept_token = !got.fresh;
                        prepared.headers.push(("Authorization".into(), got.token.header()));
                    }
                    Err(e) => return Exchange { id, request, sent: None, result: Err(e), checks: Vec::new(), captures: set_by_script, logs },
                }
            }
            // AWS: signed as of now, after everything else is in place; a header the user wrote wins.
            if let Some(aws) = prepared.aws.clone()
                && prepared.header("authorization").is_none()
            {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_default();
                if let Err(e) = sigv4::sign(&mut prepared, &aws, now) {
                    return Exchange { id, request, sent: None, result: Err(Error::Invalid(e)), checks: Vec::new(), captures: set_by_script, logs };
                }
            }
            let options = self.options.for_request(&prepared);
            let mut result = http::send(&prepared, &options).await;
            // A kept token the server no longer takes (revoked, the server restarted): once more
            // with a renewed one.
            if let Some(o) = &oauth
                && kept_token
                && result.as_ref().is_ok_and(|r| r.status == 401)
            {
                match oauth::token(o, &self.tokens, &self.options, Mode::Renew).await {
                    Ok(got) => {
                        logs.push("OAuth 2.0: сервер не принял сохранённый токен (401), запрос отправлен ещё раз с новым".into());
                        logs.extend(got.note);
                        set_authorization(&mut prepared, got.token.header());
                        result = http::send(&prepared, &options).await;
                    }
                    Err(e) => logs.push(format!("сервер не принял сохранённый токен (401), а новый не получен: {e}")),
                }
            }
            // Digest: the first answer is the server's challenge; the request goes again with
            // the answer to it.
            if let Some((username, password)) = prepared.digest.clone()
                && prepared.header("authorization").is_none()
                && let Ok(response) = &result
                && response.status == 401
                && let Some(challenge) = digest::challenge(&response.headers)
            {
                let uri = &prepared.url[url::Position::BeforePath..url::Position::AfterQuery];
                let cnonce = uuid::Uuid::new_v4().simple().to_string();
                match digest::answer(challenge, &prepared.method, uri, &prepared.body, &username, &password, &cnonce) {
                    Ok(header) => {
                        set_authorization(&mut prepared, header);
                        result = http::send(&prepared, &options).await;
                    }
                    Err(e) => logs.push(e),
                }
            }
            // What the jar added goes into what was sent, so the window shows it.
            if let Ok(Some(cookie)) = result.as_ref().map(|r| r.sent_cookie.clone()) {
                prepared.headers.retain(|(n, _)| !n.eq_ignore_ascii_case("cookie"));
                prepared.headers.push(("Cookie".into(), cookie));
            }
            let (checks, mut captures) = match &result {
                Ok(response) => self.read(&scripted, &prepared, response, &dir, &mut logs),
                Err(_) => (Vec::new(), Vec::new()),
            };
            set_by_script.retain(|c| !captures.iter().any(|x| x.name == c.name));
            captures.splice(0..0, set_by_script);
            Exchange { id, request, sent: Some(prepared), result, checks, captures, logs }
        })
    }

    /// Sends the requests that capture the missing variables; whether any went.
    async fn send_producers(&mut self, missing: &[String], id: Option<&str>, depth: usize, before: &mut Vec<Exchange>) -> bool {
        let mut sent_any = false;
        for name in missing {
            // An earlier request of this loop may have brought it on the way.
            if self.captured.contains_key(name) {
                continue;
            }
            let Some(source) = self.producers.get(name) else { continue };
            if Some(source.id.as_str()) == id || self.failed.contains(&source.id) {
                continue;
            }
            let source = source.clone();
            let done = self.exchange(Some(source.id.clone()), source.request, source.dir, depth + 1, before).await;
            if done.result.is_err() || done.captures.iter().any(|c| c.error.is_some()) {
                self.failed.insert(source.id);
            }
            before.push(done);
            sent_any = true;
        }
        sent_any
    }

    /// Captures first, then the script after, then the checks, so each can use what the one
    /// before it set.
    fn read(&mut self, request: &Request, sent: &Prepared, response: &Response, dir: &Path, logs: &mut Vec<String>) -> (Vec<CheckResult>, Vec<Captured>) {
        let answer = Answer::new(response);
        let mut captures = checks::capture(&request.captures, &answer);
        for c in &captures {
            if let Some(value) = &c.value {
                self.captured.insert(c.name.clone(), value.clone());
            }
        }
        let mut results = Vec::new();
        if !request.scripts.after.trim().is_empty() {
            let out = scripts::after(&request.scripts.after, sent, response, &self.vars());
            logs.extend(out.logs);
            for (name, value) in out.vars {
                self.captured.insert(name.clone(), value.clone());
                captures.retain(|c| c.name != name);
                captures.push(Captured { name, value: Some(value), error: None });
            }
            results.extend(out.tests.into_iter().map(|t| CheckResult { check: t.name, ok: t.ok, actual: None, error: t.error }));
            if let Some(e) = out.error {
                results.push(CheckResult { check: "скрипт после ответа".into(), ok: false, actual: None, error: Some(e) });
            }
        }
        results.extend(checks::run_checks(&request.checks, &answer, &self.vars(), Some(dir)));
        (results, captures)
    }
}

/// A certificate of the collection with variables substituted; a missing one is its error.
fn certificate(spec: &CertSpec, vars: &Vars, root: &Path) -> ClientCert {
    let rendered = (|| {
        Ok::<_, Error>(CertSpec {
            host: render(&spec.host, vars)?,
            cert: render(&spec.cert, vars)?,
            key: render(&spec.key, vars)?,
            passphrase: render(&spec.passphrase, vars)?,
        })
    })();
    match rendered {
        Ok(spec) => ClientCert::load(&spec, root),
        Err(e) => ClientCert { host: spec.host.clone(), identity: Err(e.to_string()) },
    }
}

fn set_authorization(prepared: &mut Prepared, value: String) {
    prepared.headers.retain(|(n, _)| !n.eq_ignore_ascii_case("authorization"));
    prepared.headers.push(("Authorization".into(), value));
}

async fn attempt(mut runner: Runner, step: Step, i: usize, pause: Duration) -> (usize, Runner, Vec<Exchange>) {
    if !pause.is_zero() {
        tokio::time::sleep(pause).await;
    }
    let exchanges = runner.send(Some(&step.id), &step.request, &step.dir).await;
    (i, runner, exchanges)
}
