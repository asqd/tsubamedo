//! Requests that need each other. A request keeps values from its response (`[capture]
//! token = "$.token"`), and others use them as plain `{{token}}`. When a request is sent and
//! such a variable is not set yet, the request that captures it goes first; a collection run
//! orders requests so that those go first anyway.
//!
//! Captured values sit above the collection's files and below the caller's overrides
//! (`--var` in the CLI): a fresh token beats the placeholder in `dev.local.toml`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::checks::{self, Answer, CheckResult, Captured};
use crate::collection::{Collection, Entry};
use crate::http::{self, Response, SendOptions};
use crate::prepare::{Prepared, prepare, variables};
use crate::{Error, Request, Result, Vars};

/// How deep requests may pull in other requests.
const MAX_CHAIN: usize = 8;

/// A request that captures a variable.
#[derive(Debug, Clone)]
pub struct Source {
    pub id: String,
    pub request: Request,
    pub dir: PathBuf,
}

/// Which request captures each variable; the first in the collection's order wins.
pub fn producers(collection: &Collection) -> BTreeMap<String, Source> {
    let mut out = BTreeMap::new();
    for entry in &collection.requests {
        for capture in entry.request.captures.iter().filter(|c| c.enabled) {
            out.entry(capture.name.trim().to_string()).or_insert_with(|| Source {
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

pub struct Runner {
    base: Vars,
    overrides: Vars,
    /// Values captured from responses so far.
    pub captured: Vars,
    producers: BTreeMap<String, Source>,
    /// Requests that did not give their values; they are not sent again in this run.
    failed: BTreeSet<String>,
    options: SendOptions,
}

impl Runner {
    pub fn new(collection: Option<&Collection>, env: Option<&str>, overrides: Vars, captured: Vars, options: SendOptions) -> Result<Self> {
        let (base, producers) = match collection {
            Some(c) => (c.vars(env, &Vars::new())?, producers(c)),
            None => (Vars::new(), BTreeMap::new()),
        };
        Ok(Runner { base, overrides, captured, producers, failed: BTreeSet::new(), options })
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
            let mut prepared = prepare(&request, &self.vars(), &dir);
            if let Err(Error::UndefinedVars(missing)) = &prepared
                && depth < MAX_CHAIN
            {
                let mut sent_any = false;
                for name in missing {
                    // An earlier request of this loop may have brought it on the way.
                    if self.captured.contains_key(name) {
                        continue;
                    }
                    let Some(source) = self.producers.get(name) else { continue };
                    if Some(&source.id) == id.as_ref() || self.failed.contains(&source.id) {
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
                if sent_any {
                    prepared = prepare(&request, &self.vars(), &dir);
                }
            }
            let prepared = match prepared {
                Ok(p) => p,
                Err(e) => return Exchange { id, request, sent: None, result: Err(e), checks: Vec::new(), captures: Vec::new() },
            };
            let options = self.options.for_request(&prepared);
            let result = http::send(&prepared, &options).await;
            let (checks, captures) = match &result {
                Ok(response) => self.read(&request, response),
                Err(_) => (Vec::new(), Vec::new()),
            };
            Exchange { id, request, sent: Some(prepared), result, checks, captures }
        })
    }

    /// Captures first, so checks can use what was just captured.
    fn read(&mut self, request: &Request, response: &Response) -> (Vec<CheckResult>, Vec<Captured>) {
        let answer = Answer::new(response);
        let captures = checks::capture(&request.captures, &answer);
        for c in &captures {
            if let Some(value) = &c.value {
                self.captured.insert(c.name.clone(), value.clone());
            }
        }
        let checks = checks::run_checks(&request.checks, &answer, &self.vars());
        (checks, captures)
    }
}
