//! Everything the app's window can ask for, behind one call: `call("send", {...})`.
//!
//! The Tauri shell and the dev bridge (the interface in a plain browser) both forward to
//! [`Session::call`], so they cannot drift apart. Arguments and results are JSON in camelCase,
//! mirrored by `apps/desktop/src/api.ts`.

pub mod history;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::task::AbortHandle;
use tsubamedo_core::checks::{CheckResult, Captured};
use tsubamedo_core::collection::{Collection, VarSource};
use tsubamedo_core::run::{self, Exchange, Runner};
use tsubamedo_core::http::{Redirect, Response, ResponseBody, SendOptions, Timings, TlsInfo};
use tsubamedo_core::organize;
use tsubamedo_core::prepare::prepare;
use tsubamedo_core::request_file::RequestFile;
use tsubamedo_core::view::{Matcher, TextView};
use tsubamedo_core::{Request, Vars};

/// Responses kept for the viewer; older ones are dropped with their temporary files.
const KEEP_RESPONSES: usize = 30;
/// `bodyText` refuses more than this: copying 300 MB into the clipboard helps nobody.
const TEXT_LIMIT: u64 = 20 << 20;

#[derive(Default)]
pub struct Session {
    state: Mutex<State>,
    sends: Mutex<HashMap<String, AbortHandle>>,
    /// Where history is kept; without it nothing is remembered between runs.
    data_dir: Option<PathBuf>,
}

#[derive(Default)]
struct State {
    collection: Option<Collection>,
    /// Bodies of recent responses (sent now or opened from history) for the viewer.
    responses: VecDeque<(u64, ResponseBody)>,
    /// View id → the response it shows and the view.
    views: HashMap<u64, (u64, Arc<TextView>)>,
    /// `(response, pretty)` → view, so switching modes back and forth costs nothing.
    view_of: HashMap<(u64, bool), u64>,
    /// Values captured from responses for this collection, by environment.
    captured: history::CapturedStore,
    next_id: u64,
}

impl State {
    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn body(&self, id: u64) -> Result<ResponseBody, String> {
        self.responses
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, r)| r.clone())
            .ok_or_else(|| "ответ уже удалён из памяти, отправьте запрос ещё раз".into())
    }

    fn collection(&self) -> Result<&Collection, String> {
        self.collection.as_ref().ok_or_else(|| "коллекция не открыта".into())
    }

    /// Keeps a body for the viewer and drops the oldest ones with their views.
    fn keep_body(&mut self, body: ResponseBody) -> u64 {
        let id = self.id();
        self.responses.push_back((id, body));
        while self.responses.len() > KEEP_RESPONSES {
            let (old, _) = self.responses.pop_front().unwrap();
            self.drop_views(old);
        }
        id
    }

    /// Drops the bodies the window no longer shows, with their views and temporary files.
    /// Only those up to `seen`: newer ones are on their way to a tab.
    fn retain(&mut self, shown: &[u64], seen: u64) {
        let gone: Vec<u64> = self.responses.iter().map(|&(id, _)| id).filter(|id| *id <= seen && !shown.contains(id)).collect();
        self.responses.retain(|(id, _)| !gone.contains(id));
        for &id in &gone {
            self.drop_views(id);
        }
        if !gone.is_empty() {
            give_back_memory();
        }
    }

    fn drop_views(&mut self, response: u64) {
        self.views.retain(|_, (of, _)| *of != response);
        self.view_of.retain(|(of, _), _| *of != response);
    }
}

/// Returns freed memory to the system. The allocator keeps what a big view used for the next
/// one, tens of megabytes that the window would carry while idle.
fn give_back_memory() {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        // SAFETY: a null zone means all zones; the call only releases free pages.
        unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    }
}

type Reply = Result<Value, String>;

fn args<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| format!("неверные аргументы: {e}"))
}

fn reply<T: Serialize>(value: T) -> Reply {
    serde_json::to_value(value).map_err(|e| e.to_string())
}

/// Blocking work (big files, formatting, search) off the async threads.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

impl Session {
    /// A session that remembers nothing between runs (tests).
    pub fn new() -> Arc<Self> {
        Arc::new(Session::default())
    }

    /// A session keeping history in `data_dir` (the app's data folder).
    pub fn with_data_dir(data_dir: PathBuf) -> Arc<Self> {
        let session = Session { data_dir: Some(data_dir), ..Session::default() };
        if let Some(history) = session.history() {
            session.state.lock().unwrap().captured = history.captured();
        }
        Arc::new(session)
    }

    fn history(&self) -> Option<history::History> {
        let data = self.data_dir.as_ref()?;
        let root = self.state.lock().unwrap().collection.as_ref().map(|c| c.root.clone());
        Some(history::History::open(data, root.as_deref()))
    }

    pub async fn call(self: Arc<Self>, method: &str, a: Value) -> Reply {
        match method {
            "openCollection" => self.open_collection(args::<PathArg>(a)?.path),
            "reloadCollection" => {
                let root = self.state.lock().unwrap().collection()?.root.clone();
                self.open_collection(root.to_string_lossy().into_owned())
            }
            "getRequest" => self.get_request(args::<IdArg>(a)?.id),
            "saveRequest" => self.save_request(args(a)?),
            "createRequest" => self.create_request(args(a)?),
            "send" => self.send(args(a)?).await,
            "runCollection" => self.run_collection(args(a)?).await,
            "validateCheck" => Ok(match tsubamedo_core::checks::validate(&args::<TextArg>(a)?.text) {
                Ok(()) => Value::Null,
                Err(e) => Value::String(e),
            }),
            "cancel" => {
                if let Some(handle) = self.sends.lock().unwrap().remove(&args::<SendIdArg>(a)?.send_id) {
                    handle.abort();
                }
                Ok(Value::Null)
            }
            "retain" => {
                let a: RetainArg = args(a)?;
                self.state.lock().unwrap().retain(&a.responses, a.seen);
                Ok(Value::Null)
            }
            "vars" => self.vars(args::<EnvArg>(a)?.env),
            "importCurl" => reply(tsubamedo_core::curl::parse(&args::<TextArg>(a)?.text).map_err(|e| e.to_string())?),
            "curl" => self.curl(args(a)?),
            "view" => self.view(args(a)?).await,
            "viewLines" => self.view_lines(args(a)?),
            "viewSearch" => self.view_search(args(a)?).await,
            "viewFolds" => self.view_folds(args(a)?),
            "bodyText" => self.body_text(args(a)?).await,
            "saveBody" => self.save_body(args(a)?).await,
            "renameRequest" => {
                let a: RenameArg = args(a)?;
                self.reorganize(Some(a.id.clone()), |root| organize::rename_request(root, &a.id, &a.name))
            }
            "moveRequest" => {
                let a: MoveArg = args(a)?;
                self.reorganize(Some(a.id.clone()), |root| organize::move_request(root, &a.id, &a.folder))
            }
            "duplicateRequest" => {
                let a: IdArg = args(a)?;
                self.reorganize(None, |root| organize::duplicate_request(root, &a.id))
            }
            "deleteRequest" => {
                let a: IdArg = args(a)?;
                self.reorganize(None, |root| organize::delete_request(root, &a.id).map(|()| String::new()))
            }
            "createFolder" => {
                let a: FolderNameArg = args(a)?;
                self.reorganize(None, |root| organize::create_folder(root, &a.folder, &a.name))
            }
            "renameFolder" => {
                let a: FolderNameArg = args(a)?;
                self.reorganize(Some(a.folder.clone()), |root| organize::rename_folder(root, &a.folder, &a.name))
            }
            "moveFolder" => {
                let a: MoveFolderArg = args(a)?;
                self.reorganize(Some(a.folder.clone()), |root| organize::move_folder(root, &a.folder, &a.parent))
            }
            "deleteFolder" => {
                let a: FolderArg = args(a)?;
                self.reorganize(None, |root| organize::delete_folder(root, &a.folder).map(|()| String::new()))
            }
            "history" => self.list_history(args(a)?),
            "openHistory" => self.open_history(args::<IdArg>(a)?.id),
            "clearHistory" => {
                if let Some(h) = self.history() {
                    h.clear().map_err(|e| e.to_string())?;
                }
                self.state.lock().unwrap().captured.clear();
                Ok(Value::Null)
            }
            other => Err(format!("неизвестная команда {other}")),
        }
    }

    fn open_collection(&self, path: String) -> Reply {
        let path = PathBuf::from(path);
        let collection = Collection::open(&path).map_err(|e| e.to_string())?;
        let info = collection_info(&collection);
        let captured = self.data_dir.as_ref().map(|data| history::History::open(data, Some(&collection.root)).captured());
        let mut state = self.state.lock().unwrap();
        state.collection = Some(collection);
        state.captured = captured.unwrap_or_default();
        reply(info)
    }

    fn get_request(&self, id: String) -> Reply {
        let state = self.state.lock().unwrap();
        let entry = state.collection()?.find(&id).ok_or_else(|| format!("запроса {id} нет в коллекции"))?;
        reply(&entry.request)
    }

    fn save_request(&self, a: SaveArg) -> Reply {
        let mut state = self.state.lock().unwrap();
        let collection = state.collection.as_mut().ok_or("коллекция не открыта")?;
        let entry = collection.requests.iter_mut().find(|e| e.id == a.id).ok_or_else(|| format!("запроса {} нет в коллекции", a.id))?;
        let mut file = RequestFile::open(&entry.path).map_err(|e| e.to_string())?;
        file.update(&a.request);
        let written = file.save().map_err(|e| e.to_string())?;
        entry.request = a.request;
        Ok(json!({ "written": written }))
    }

    fn create_request(&self, a: CreateArg) -> Reply {
        let root = self.state.lock().unwrap().collection()?.root.clone();
        let folder = root.join(a.folder.trim_matches('/'));
        let base: String = a.name.chars().map(|c| if "/\\:*?\"<>|".contains(c) { '-' } else { c }).collect();
        let base = base.trim().trim_start_matches('.').to_string();
        let base = if base.is_empty() { "Новый запрос".to_string() } else { base };
        let mut path = folder.join(format!("{base}.toml"));
        let mut n = 2;
        while path.exists() {
            path = folder.join(format!("{base} {n}.toml"));
            n += 1;
        }
        let request = Request { name: Some(a.name.trim().to_string()).filter(|n| !n.is_empty()), ..Request::default() };
        RequestFile::create(&path, &request).save().map_err(|e| e.to_string())?;
        let info = self.open_collection(root.to_string_lossy().into_owned())?;
        let id = path.strip_prefix(&root).unwrap_or(&path).with_extension("");
        let id = id.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        Ok(json!({ "id": id, "collection": info }))
    }

    /// A file operation on the collection, then the collection read again. `old` is the id the
    /// operation renames, so history follows the request to its new name.
    fn reorganize(&self, old: Option<String>, op: impl FnOnce(&Path) -> tsubamedo_core::Result<String>) -> Reply {
        let root = self.state.lock().unwrap().collection()?.root.clone();
        let new = op(&root).map_err(|e| e.to_string())?;
        if let (Some(old), Some(history)) = (old, self.history())
            && old != new
        {
            history.rename(&old, &new).map_err(|e| e.to_string())?;
        }
        let info = self.open_collection(root.to_string_lossy().into_owned())?;
        Ok(json!({ "id": new, "collection": info }))
    }

    fn list_history(&self, a: HistoryArg) -> Reply {
        let Some(history) = self.history() else { return Ok(json!([])) };
        let entries: Vec<Value> = history
            .list(a.request_id.as_deref(), 300)
            .into_iter()
            .map(|e| {
                json!({
                    "id": e.id,
                    "at": e.at,
                    "requestId": e.request_id,
                    "env": e.env,
                    "method": e.request.method.to_ascii_uppercase(),
                    "url": e.request.url,
                    "name": e.request.name,
                    "status": e.status,
                    "error": e.error,
                    "totalMs": e.total_ms,
                    "size": e.size,
                    "hasBody": e.has_body,
                })
            })
            .collect();
        Ok(Value::Array(entries))
    }

    /// An old exchange back in the window: its request and, when the body was kept, its answer.
    fn open_history(&self, id: String) -> Reply {
        let history = self.history().ok_or("история не ведётся")?;
        let (entry, body) = history.get(&id).ok_or("этой записи в истории уже нет")?;
        let response = match (entry.meta, body) {
            (Some(mut meta), Some(body)) => {
                let response_id = self.state.lock().unwrap().keep_body(ResponseBody::Memory(body.into()));
                meta["responseId"] = json!(response_id);
                meta
            }
            _ => Value::Null,
        };
        Ok(json!({
            "requestId": entry.request_id,
            "env": entry.env,
            "request": entry.request,
            "response": response,
            "status": entry.status,
            "error": entry.error,
            "at": entry.at,
        }))
    }

    fn record(&self, request_id: Option<&str>, env: Option<&str>, request: &Request, outcome: Result<(&ResponseMeta, &ResponseBody), &str>) {
        let Some(history) = self.history() else { return };
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let id = format!("{at}-{}", self.state.lock().unwrap().id());
        let mut entry = history::Entry {
            id,
            at,
            request_id: request_id.map(str::to_string),
            env: env.map(str::to_string),
            request: request.clone(),
            status: None,
            error: None,
            total_ms: None,
            size: None,
            has_body: false,
            meta: None,
        };
        let mut body_bytes = None;
        match outcome {
            Ok((meta, body)) => {
                entry.status = Some(meta.status);
                entry.total_ms = Some(meta.timings.total.as_secs_f64() * 1000.0);
                entry.size = Some(body.len());
                let mut value = serde_json::to_value(meta).unwrap_or(Value::Null);
                history::mask_secrets(&mut value);
                entry.meta = Some(value);
                if body.len() <= history::BODY_LIMIT {
                    let mut bytes = Vec::new();
                    if body.reader().and_then(|mut r| std::io::copy(&mut r, &mut bytes)).is_ok() {
                        entry.has_body = true;
                        body_bytes = Some(bytes);
                    }
                }
            }
            Err(error) => entry.error = Some(error.to_string()),
        }
        // History is a convenience: failing to write it must not fail the request.
        let _ = history.add(&entry, body_bytes.as_deref());
    }

    /// An exchange into history; a request stopped before sending (a missing variable) is not.
    fn record_exchange(&self, ex: &Exchange, env: Option<&str>, meta: Option<&ResponseMeta>) {
        match (&ex.result, meta) {
            (Ok(response), Some(meta)) => self.record(ex.id.as_deref(), env, &ex.request, Ok((meta, &response.body))),
            (Ok(response), None) => {
                let meta = ResponseMeta::new(0, response, ex);
                self.record(ex.id.as_deref(), env, &ex.request, Ok((&meta, &response.body)));
            }
            (Err(e), _) if ex.sent.is_some() => self.record(ex.id.as_deref(), env, &ex.request, Err(&e.to_string())),
            _ => {}
        }
    }

    /// A runner over the open collection with what earlier responses captured.
    fn runner(&self, env: Option<&str>, insecure: bool) -> Result<Runner, String> {
        let state = self.state.lock().unwrap();
        let captured: Vars =
            state.captured.get(env.unwrap_or_default()).map(|m| m.iter().map(|(k, v)| (k.clone(), v.value.clone())).collect()).unwrap_or_default();
        let options = SendOptions { verify_tls: !insecure, ..SendOptions::default() };
        Runner::new(state.collection.as_ref(), env, Vars::new(), captured, options).map_err(|e| e.to_string())
    }

    /// Keeps what the exchanges captured, for the next sends and the next start.
    fn keep_captured(&self, env: Option<&str>, exchanges: &[Exchange]) {
        let mut state = self.state.lock().unwrap();
        let store = state.captured.entry(env.unwrap_or_default().to_string()).or_default();
        let mut changed = false;
        for ex in exchanges {
            for c in &ex.captures {
                if let Some(value) = &c.value {
                    let from = ex.id.clone().unwrap_or_else(|| "черновик".into());
                    changed |= store.insert(c.name.clone(), history::CapturedValue { value: value.clone(), from }).is_none_or(|old| old.value != *value);
                }
            }
        }
        let store = state.captured.clone();
        drop(state);
        if changed && let Some(history) = self.history() {
            let _ = history.save_captured(&store);
        }
    }

    /// Variables for hints in the editor: from the files, then those captured from responses
    /// (they win), then those a request will capture when it is sent.
    fn vars(&self, env: Option<String>) -> Reply {
        let state = self.state.lock().unwrap();
        let mut list = match &state.collection {
            Some(c) => c.var_sources(env.as_deref()).map_err(|e| e.to_string())?,
            None => Vec::new(),
        };
        if let Some(captured) = state.captured.get(env.as_deref().unwrap_or_default()) {
            for (name, v) in captured {
                list.retain(|x| x.name != *name);
                list.push(VarSource { name: name.clone(), value: v.value.clone(), source: format!("из ответа «{}»", v.from), secret: false });
            }
        }
        if let Some(c) = &state.collection {
            for (name, source) in run::producers(c) {
                if !list.iter().any(|x| x.name == name) {
                    list.push(VarSource { name, value: String::new(), source: format!("из ответа «{}», придёт при отправке", source.id), secret: false });
                }
            }
        }
        reply(list)
    }

    /// Variables and the folder for relative paths of a request about to be sent.
    fn context(&self, id: Option<&str>, env: Option<&str>) -> Result<(Vars, PathBuf), String> {
        let state = self.state.lock().unwrap();
        Ok(match &state.collection {
            Some(c) => {
                let vars = c.vars(env, &Vars::new()).map_err(|e| e.to_string())?;
                let dir = id.and_then(|id| c.find(id)).map_or_else(|| c.root.clone(), |e| e.dir().to_path_buf());
                (vars, dir)
            }
            None => (Vars::new(), dirs_home()),
        })
    }

    fn curl(&self, a: CurlArg) -> Reply {
        let (vars, dir) = self.context(a.id.as_deref(), a.env.as_deref())?;
        let prepared = prepare(&a.request, &vars, &dir).map_err(|e| e.to_string())?;
        Ok(Value::String(tsubamedo_core::curl::to_curl(&prepared, &a.request.body, &vars)))
    }

    /// Sends the request, first those whose captured values it needs and does not have yet.
    async fn send(self: Arc<Self>, a: SendArg) -> Reply {
        let mut runner = self.runner(a.env.as_deref(), a.insecure)?;
        let (_, dir) = self.context(a.id.as_deref(), a.env.as_deref())?;
        let (id, request) = (a.id.clone(), a.request.clone());
        let task = tokio::spawn(async move { runner.send(id.as_deref(), &request, &dir).await });
        self.sends.lock().unwrap().insert(a.send_id.clone(), task.abort_handle());
        let result = task.await;
        self.sends.lock().unwrap().remove(&a.send_id);
        let mut exchanges = match result {
            Ok(exchanges) => exchanges,
            Err(e) if e.is_cancelled() => return Err("Отменено".into()),
            Err(e) => return Err(e.to_string()),
        };
        self.keep_captured(a.env.as_deref(), &exchanges);
        let last = exchanges.pop().expect("the request itself is always there");
        for ex in &exchanges {
            self.record_exchange(ex, a.env.as_deref(), None);
        }
        let before: Vec<Before> = exchanges.iter().map(Before::of).collect();
        let response = match &last.result {
            Ok(response) => response,
            Err(e) => {
                self.record_exchange(&last, a.env.as_deref(), None);
                let mut message = e.to_string();
                for b in before.iter().filter(|b| !b.passed) {
                    message.push_str(&format!("\nСначала отправлен «{}»: {}", b.id, b.failures.join("; ")));
                }
                return Err(message);
            }
        };
        let id = self.state.lock().unwrap().keep_body(response.body.clone());
        let mut meta = ResponseMeta::new(id, response, &last);
        meta.before = before;
        self.record_exchange(&last, a.env.as_deref(), Some(&meta));
        reply(meta)
    }

    /// Runs the collection or one folder of it, each request after those it takes values from.
    async fn run_collection(self: Arc<Self>, a: RunArg) -> Reply {
        let mut runner = self.runner(a.env.as_deref(), a.insecure)?;
        let plan: Vec<(String, Request, PathBuf)> = {
            let state = self.state.lock().unwrap();
            let collection = state.collection()?;
            let inside = |id: &str| a.folder.as_deref().is_none_or(|f| f.is_empty() || id.starts_with(&format!("{f}/")));
            runner.order(collection, |e| inside(&e.id)).into_iter().map(|e| (e.id.clone(), e.request.clone(), e.dir().to_path_buf())).collect()
        };
        let started = std::time::Instant::now();
        let task = tokio::spawn(async move {
            let mut all = Vec::new();
            for (id, request, dir) in plan {
                all.extend(runner.send(Some(&id), &request, &dir).await);
            }
            all
        });
        self.sends.lock().unwrap().insert(a.send_id.clone(), task.abort_handle());
        let result = task.await;
        self.sends.lock().unwrap().remove(&a.send_id);
        let exchanges = match result {
            Ok(exchanges) => exchanges,
            Err(e) if e.is_cancelled() => return Err("Отменено".into()),
            Err(e) => return Err(e.to_string()),
        };
        self.keep_captured(a.env.as_deref(), &exchanges);
        let results: Vec<Value> = exchanges
            .iter()
            .map(|ex| {
                self.record_exchange(ex, a.env.as_deref(), None);
                let response = ex.result.as_ref().ok();
                json!({
                    "id": ex.id,
                    "method": ex.request.method.to_ascii_uppercase(),
                    "status": response.map(|r| r.status),
                    "totalMs": response.map(|r| r.timings.total.as_secs_f64() * 1000.0),
                    "size": response.map(|r| r.body.len()),
                    "passed": ex.passed(),
                    "failures": ex.failures(),
                    "checks": ex.checks.len(),
                    "checksPassed": ex.checks.iter().filter(|c| c.ok).count(),
                })
            })
            .collect();
        Ok(json!({ "results": results, "totalMs": started.elapsed().as_secs_f64() * 1000.0 }))
    }

    async fn view(self: Arc<Self>, a: ViewArg) -> Reply {
        let (body, cached) = {
            let state = self.state.lock().unwrap();
            let cached = state.view_of.get(&(a.response_id, a.pretty)).and_then(|id| Some((*id, state.views.get(id)?.1.clone())));
            (state.body(a.response_id)?, cached)
        };
        let (id, view, note) = match cached {
            Some((id, view)) => (id, view, None),
            None => {
                let pretty = a.pretty;
                let (view, note) = blocking(move || {
                    let raw = || TextView::raw(&body).map_err(|e| e.to_string());
                    if !pretty {
                        return Ok((raw()?, None));
                    }
                    let defaults = SendOptions::default();
                    match TextView::pretty_json(&body, defaults.spill_threshold, defaults.spill_dir) {
                        Ok(Ok(view)) => Ok((view, None)),
                        Ok(Err(reason)) => Ok((raw()?, Some(format!("Не JSON ({reason}), показано как есть")))),
                        Err(e) => Err(e.to_string()),
                    }
                })
                .await?;
                let view = Arc::new(view);
                let mut state = self.state.lock().unwrap();
                let id = state.id();
                state.views.insert(id, (a.response_id, view.clone()));
                // A failed pretty view is the raw one; it is not cached, so the note shows again.
                if note.is_none() {
                    state.view_of.insert((a.response_id, a.pretty), id);
                }
                (id, view, note)
            }
        };
        Ok(json!({
            "viewId": id,
            "lines": view.line_count(),
            "bytes": view.len(),
            "pretty": view.pretty,
            "note": note,
        }))
    }

    fn get_view(&self, id: u64) -> Result<Arc<TextView>, String> {
        self.state.lock().unwrap().views.get(&id).map(|(_, view)| view.clone()).ok_or_else(|| "вид ответа уже закрыт".into())
    }

    fn view_lines(&self, a: LinesArg) -> Reply {
        let view = self.get_view(a.view_id)?;
        let matcher = a.query.as_deref().and_then(|q| Matcher::new(q, a.case_sensitive));
        let lines: Vec<LineOut> = a
            .lines
            .iter()
            .filter(|&&n| (n as usize) < view.line_count())
            .map(|&n| {
                let bytes = view.line(n as usize);
                let text = String::from_utf8_lossy(bytes).into_owned();
                // Offsets for JavaScript strings count UTF-16 units.
                let utf16 = |byte: usize| text.get(..byte).map_or(0, |s| s.encode_utf16().count()) as u32;
                let marks = matcher.as_ref().map_or_else(Vec::new, |m| {
                    m.ranges(text.as_bytes()).into_iter().map(|(s, e)| [utf16(s), utf16(e)]).collect()
                });
                LineOut { n, fold_end: view.fold_end(n), marks, text }
            })
            .collect();
        reply(lines)
    }

    fn view_folds(&self, a: ViewIdArg) -> Reply {
        const LIMIT: usize = 100_000;
        let view = self.get_view(a.view_id)?;
        let folds = view
            .top_folds(LIMIT)
            .ok_or("внутри больше 100 000 объектов — сворачивать их все нет смысла")?;
        reply(folds.into_iter().flat_map(|(o, c)| [o, c]).collect::<Vec<u32>>())
    }

    async fn view_search(&self, a: SearchArg) -> Reply {
        let view = self.get_view(a.view_id)?;
        let Some(matcher) = Matcher::new(&a.query, a.case_sensitive) else {
            return Ok(json!({ "total": 0, "lines": [], "capped": false }));
        };
        let found = blocking(move || Ok(view.search(&matcher))).await?;
        Ok(json!({
            "total": found.total,
            "lines": found.lines,
            "capped": found.total >= tsubamedo_core::view::MAX_MATCHES,
        }))
    }

    async fn body_text(&self, a: BodyArg) -> Reply {
        let view = match a.view_id {
            Some(id) => Some(self.get_view(id)?),
            None => None,
        };
        let body = self.state.lock().unwrap().body(a.response_id)?;
        let size = view.as_ref().map_or(body.len(), |v| v.len());
        if size > TEXT_LIMIT {
            return Err(format!("ответ слишком большой для буфера обмена ({} МБ), сохраните его в файл", size >> 20));
        }
        blocking(move || {
            let mut out = Vec::new();
            match view {
                Some(view) => view.write_to(&mut out).map_err(|e| e.to_string())?,
                None => {
                    std::io::copy(&mut body.reader().map_err(|e| e.to_string())?, &mut out).map_err(|e| e.to_string())?;
                }
            }
            Ok(Value::String(String::from_utf8_lossy(&out).into_owned()))
        })
        .await
    }

    async fn save_body(&self, a: SaveBodyArg) -> Reply {
        let view = match a.view_id {
            Some(id) => Some(self.get_view(id)?),
            None => None,
        };
        let body = self.state.lock().unwrap().body(a.response_id)?;
        blocking(move || {
            let path = Path::new(&a.path);
            let mut file = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| e.to_string())?);
            match view {
                Some(view) => view.write_to(&mut file).map_err(|e| e.to_string())?,
                None => {
                    std::io::copy(&mut body.reader().map_err(|e| e.to_string())?, &mut file).map_err(|e| e.to_string())?;
                }
            }
            Ok(Value::Null)
        })
        .await
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn collection_info(c: &Collection) -> CollectionInfo {
    CollectionInfo {
        root: c.root.to_string_lossy().into_owned(),
        name: c.name.clone(),
        environments: c.environments(),
        folders: c.folders.clone(),
        requests: c
            .requests
            .iter()
            .map(|e| RequestItem {
                id: e.id.clone(),
                name: e.display_name().rsplit('/').next().unwrap_or_default().to_string(),
                method: e.request.method.to_ascii_uppercase(),
                folder: e.id.rsplit_once('/').map(|(f, _)| f.to_string()).unwrap_or_default(),
            })
            .collect(),
        broken: c.broken.iter().map(|e| e.to_string()).collect(),
    }
}

#[derive(Deserialize)]
struct PathArg {
    path: String,
}

#[derive(Deserialize)]
struct IdArg {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendIdArg {
    send_id: String,
}

#[derive(Deserialize)]
struct RenameArg {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct MoveArg {
    id: String,
    folder: String,
}

#[derive(Deserialize)]
struct FolderArg {
    folder: String,
}

#[derive(Deserialize)]
struct FolderNameArg {
    #[serde(default)]
    folder: String,
    name: String,
}

#[derive(Deserialize)]
struct MoveFolderArg {
    folder: String,
    parent: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryArg {
    request_id: Option<String>,
}

#[derive(Deserialize)]
struct EnvArg {
    env: Option<String>,
}

#[derive(Deserialize)]
struct TextArg {
    text: String,
}

#[derive(Deserialize)]
struct CurlArg {
    id: Option<String>,
    request: Request,
    env: Option<String>,
}

#[derive(Deserialize)]
struct SaveArg {
    id: String,
    request: Request,
}

#[derive(Deserialize)]
struct CreateArg {
    #[serde(default)]
    folder: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendArg {
    send_id: String,
    /// The collection entry, for relative file paths; `None` for a scratch request.
    id: Option<String>,
    request: Request,
    env: Option<String>,
    #[serde(default)]
    insecure: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViewArg {
    response_id: u64,
    pretty: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunArg {
    send_id: String,
    /// A folder of the collection; the whole collection when empty.
    folder: Option<String>,
    env: Option<String>,
    #[serde(default)]
    insecure: bool,
}

#[derive(Deserialize)]
struct RetainArg {
    responses: Vec<u64>,
    seen: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViewIdArg {
    view_id: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinesArg {
    view_id: u64,
    lines: Vec<u32>,
    query: Option<String>,
    #[serde(default)]
    case_sensitive: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchArg {
    view_id: u64,
    query: String,
    #[serde(default)]
    case_sensitive: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BodyArg {
    response_id: u64,
    view_id: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveBodyArg {
    response_id: u64,
    view_id: Option<u64>,
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CollectionInfo {
    root: String,
    name: String,
    environments: Vec<String>,
    folders: Vec<String>,
    requests: Vec<RequestItem>,
    broken: Vec<String>,
}

#[derive(Serialize)]
struct RequestItem {
    id: String,
    name: String,
    method: String,
    folder: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SentRequest {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body_size: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResponseMeta {
    response_id: u64,
    status: u16,
    reason: String,
    version: String,
    headers: Vec<(String, String)>,
    size: u64,
    wire_size: u64,
    timings: Timings,
    url: String,
    remote_addr: String,
    tls: Option<TlsInfo>,
    redirects: Vec<Redirect>,
    is_json: bool,
    content_type: Option<String>,
    /// Unix milliseconds.
    received_at: u64,
    sent: SentRequest,
    checks: Vec<CheckResult>,
    captures: Vec<Captured>,
    /// Requests sent first for the values this one needed.
    before: Vec<Before>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Before {
    id: String,
    status: Option<u16>,
    passed: bool,
    failures: Vec<String>,
}

impl Before {
    fn of(ex: &Exchange) -> Self {
        Before {
            id: ex.id.clone().unwrap_or_default(),
            status: ex.result.as_ref().ok().map(|r| r.status),
            passed: ex.passed(),
            failures: ex.failures(),
        }
    }
}

impl ResponseMeta {
    fn new(id: u64, r: &Response, ex: &Exchange) -> Self {
        let sent = match &ex.sent {
            Some(p) => SentRequest { method: p.method.clone(), url: p.url.to_string(), headers: p.headers.clone(), body_size: p.body.len() as u64 },
            None => SentRequest { method: String::new(), url: String::new(), headers: Vec::new(), body_size: 0 },
        };
        let sniffed_json = r.body.head(64).is_ok_and(|head| {
            let text = head.trim_ascii_start();
            text.starts_with(b"{") || text.starts_with(b"[")
        });
        ResponseMeta {
            response_id: id,
            status: r.status,
            reason: r.reason.clone(),
            version: r.version.clone(),
            headers: r.headers.clone(),
            size: r.body.len(),
            wire_size: r.wire_size,
            timings: r.timings.clone(),
            url: r.url.clone(),
            remote_addr: r.remote_addr.to_string(),
            tls: r.tls.clone(),
            redirects: r.redirects.clone(),
            is_json: r.is_json() || sniffed_json,
            content_type: r.header("content-type").map(str::to_string),
            received_at: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
            sent,
            checks: ex.checks.clone(),
            captures: ex.captures.clone(),
            before: Vec::new(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LineOut {
    n: u32,
    text: String,
    fold_end: Option<u32>,
    /// UTF-16 ranges of search matches.
    marks: Vec<[u32; 2]>,
}
