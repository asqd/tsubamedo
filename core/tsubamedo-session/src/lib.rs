//! Everything the app's window can ask for, behind one call: `call("send", {...})`.
//!
//! The Tauri shell and the dev bridge (the interface in a plain browser) both forward to
//! [`Session::call`], so they cannot drift apart. Arguments and results are JSON in camelCase,
//! mirrored by `apps/desktop/src/api.ts`.

pub mod history;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::task::AbortHandle;
use tsubamedo_core::checks::{CheckResult, Captured};
use tsubamedo_core::collection::{Collection, VarSource};
use tsubamedo_core::cookies::{Jar, SharedJar};
use tsubamedo_core::git;
use tsubamedo_core::oauth::{self, SharedTokens};
use tsubamedo_core::run::{self, Event, Exchange, RunOptions, Runner};
use tsubamedo_core::http::proxy::{self, Proxy};
use tsubamedo_core::http::{CertSpec, ClientCert, ProxyConfig, Redirect, Response, ResponseBody, SendOptions, Timings, TlsInfo};
use tsubamedo_core::organize;
use tsubamedo_core::prepare::{prepare_without_files, render_oauth};
use tsubamedo_core::request_file::RequestFile;
use tsubamedo_core::view::{Matcher, TextView};
use tsubamedo_core::{Auth, OAuth2, Request, Vars};

/// Responses kept for the viewer; older ones are dropped with their temporary files.
const KEEP_RESPONSES: usize = 30;
/// `bodyText` refuses more than this: copying 300 MB into the clipboard helps nobody.
const TEXT_LIMIT: u64 = 20 << 20;

#[derive(Default)]
pub struct Session {
    state: Mutex<State>,
    sends: Mutex<HashMap<String, AbortHandle>>,
    /// Collection runs in progress, by send id: what the window shows while they go.
    runs: Mutex<HashMap<String, Value>>,
    /// Where history is kept; without it nothing is remembered between runs.
    data_dir: Option<PathBuf>,
    /// The app's own settings, not the collection's: they belong to this machine.
    settings: Mutex<Settings>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    proxy: ProxyConfig,
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
    /// Cookies of this collection, a jar per environment (`""` without one).
    jars: BTreeMap<String, SharedJar>,
    /// OAuth 2.0 tokens of this collection.
    tokens: SharedTokens,
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

    fn jar(&mut self, env: Option<&str>) -> SharedJar {
        self.jars.entry(env.unwrap_or_default().to_string()).or_insert_with(|| Jar::default().shared()).clone()
    }

    /// What the session remembers of a collection: captured values, cookies, tokens.
    fn load(&mut self, history: Option<&history::History>) {
        let Some(h) = history else {
            self.captured = Default::default();
            self.jars = Default::default();
            self.tokens = Default::default();
            return;
        };
        self.captured = h.captured();
        self.jars = h.cookies().into_iter().map(|(env, jar)| (env, jar.shared())).collect();
        self.tokens = h.tokens().shared();
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
        let settings = std::fs::read(data_dir.join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let session = Session { data_dir: Some(data_dir), settings: Mutex::new(settings), ..Session::default() };
        let history = session.history();
        session.state.lock().unwrap().load(history.as_ref());
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
            "runProgress" => {
                let id = args::<SendIdArg>(a)?.send_id;
                Ok(self.runs.lock().unwrap().get(&id).cloned().unwrap_or(Value::Null))
            }
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
            "importPreview" => self.import_preview(args::<PathArg>(a)?.path),
            "importCollection" => self.import_collection(args(a)?),
            "importCurl" => reply(tsubamedo_core::curl::parse(&args::<TextArg>(a)?.text).map_err(|e| e.to_string())?),
            "curl" | "code" => self.code(args(a)?),
            "languages" => reply(tsubamedo_core::codegen::LANGUAGES),
            "fakeNames" => reply(tsubamedo_core::fake::NAMES),
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
                self.state.lock().unwrap().load(None);
                Ok(Value::Null)
            }
            "cookies" => {
                let jar = self.state.lock().unwrap().jar(args::<EnvArg>(a)?.env.as_deref());
                let list = jar.lock().unwrap().cookies();
                reply(list)
            }
            "deleteCookie" => {
                let a: CookieArg = args(a)?;
                let jar = self.state.lock().unwrap().jar(a.env.as_deref());
                jar.lock().unwrap().remove(&a.domain, &a.path, &a.name);
                self.save_stores();
                Ok(Value::Null)
            }
            "clearCookies" => {
                let jar = self.state.lock().unwrap().jar(args::<EnvArg>(a)?.env.as_deref());
                jar.lock().unwrap().clear();
                self.save_stores();
                Ok(Value::Null)
            }
            "proxySettings" => {
                let proxy = self.settings.lock().unwrap().proxy.clone();
                let system = blocking(|| Ok(proxy::system())).await?;
                Ok(json!({ "proxy": proxy, "system": system }))
            }
            "saveProxy" => {
                let a: ProxyArg = args(a)?;
                if let ProxyConfig::Custom { url, .. } = &a.proxy
                    && !url.trim().is_empty()
                {
                    Proxy::parse(url)?;
                }
                let mut settings = self.settings.lock().unwrap();
                settings.proxy = a.proxy;
                if let Some(data) = &self.data_dir {
                    std::fs::create_dir_all(data).map_err(|e| e.to_string())?;
                    std::fs::write(data.join("settings.json"), serde_json::to_vec_pretty(&*settings).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                }
                Ok(Value::Null)
            }
            "compare" => self.compare(args(a)?).await,
            "certificates" => self.certificates(args::<EnvArg>(a)?.env).await,
            "saveCertificates" => {
                let a: CertificatesArg = args(a)?;
                {
                    let state = self.state.lock().unwrap();
                    state.collection()?.save_certificates(&a.certificates).map_err(|e| e.to_string())?;
                }
                self.certificates(a.env).await
            }
            "oauthStatus" => self.oauth_status(args(a)?).await,
            "oauthToken" => self.oauth_token(args(a)?).await,
            "oauthForget" => {
                let a: OAuthArg = args(a)?;
                let (o, tokens) = self.oauth_settings(&a)?;
                tokens.lock().await.forget(&o);
                self.save_stores();
                Ok(Value::Null)
            }
            "gitStatus" => {
                let root = self.collection_root()?;
                blocking(move || git_status(&root)).await
            }
            "gitFetch" => {
                let root = self.collection_root()?;
                blocking(move || {
                    if let Some(repo) = git::Repo::open(&root)? {
                        repo.fetch()?;
                    }
                    git_status(&root)
                })
                .await
            }
            "gitDiff" => {
                let (root, path) = (self.collection_root()?, args::<PathArg>(a)?.path);
                blocking(move || reply(repo(&root)?.diff(&path)?)).await
            }
            "gitCommit" => {
                let (root, a) = (self.collection_root()?, args::<CommitArg>(a)?);
                blocking(move || {
                    let hash = repo(&root)?.commit(&a.paths, &a.message)?;
                    Ok(json!({ "hash": hash, "status": git_status(&root)? }))
                })
                .await
            }
            "gitPull" => {
                let root = self.collection_root()?;
                let message = blocking({
                    let root = root.clone();
                    move || repo(&root)?.pull()
                })
                .await?;
                // The files may have changed under the window.
                let collection = self.open_collection(root.to_string_lossy().into_owned())?;
                let status = blocking(move || git_status(&root)).await?;
                Ok(json!({ "message": message, "status": status, "collection": collection }))
            }
            "gitPush" => {
                let root = self.collection_root()?;
                blocking(move || {
                    let message = repo(&root)?.push()?;
                    Ok(json!({ "message": message, "status": git_status(&root)? }))
                })
                .await
            }
            "gitInit" => {
                let root = self.collection_root()?;
                blocking(move || {
                    git::Repo::init(&root)?;
                    git_status(&root)
                })
                .await
            }
            "gitIgnoreSecrets" => {
                let root = self.collection_root()?;
                blocking(move || {
                    repo(&root)?.ignore_secrets()?;
                    git_status(&root)
                })
                .await
            }
            other => Err(format!("неизвестная команда {other}")),
        }
    }

    fn collection_root(&self) -> Result<PathBuf, String> {
        Ok(self.state.lock().unwrap().collection()?.root.clone())
    }

    fn open_collection(&self, path: String) -> Reply {
        let path = PathBuf::from(path);
        let collection = Collection::open(&path).map_err(|e| e.to_string())?;
        let info = collection_info(&collection);
        let history = self.data_dir.as_ref().map(|data| history::History::open(data, Some(&collection.root)));
        let mut state = self.state.lock().unwrap();
        // The same collection opened again keeps what is in memory: a run may be using it.
        let same = state.collection.as_ref().is_some_and(|c| c.root == collection.root);
        state.collection = Some(collection);
        if !same {
            state.load(history.as_ref());
        }
        reply(info)
    }

    /// What a file would bring, before anything is written.
    fn import_preview(&self, path: String) -> Reply {
        let imported = tsubamedo_core::import::read(Path::new(&path)).map_err(|e| e.to_string())?;
        let mut folders: Vec<&str> = imported.requests.iter().filter_map(|p| p.folder.first().map(String::as_str)).collect();
        folders.dedup();
        Ok(json!({
            "format": imported.format.title(),
            "name": imported.name,
            "requests": imported.requests.len(),
            "folders": folders.len(),
            "environments": imported.environments.iter().map(|e| &e.name).collect::<Vec<_>>(),
            "vars": imported.vars.len(),
            "warnings": imported.warnings,
        }))
    }

    /// Into the open collection (a folder in it), or a new collection in `parent`, then opened.
    fn import_collection(&self, a: ImportArg) -> Reply {
        let imported = tsubamedo_core::import::read(Path::new(&a.path)).map_err(|e| e.to_string())?;
        let dir = match a.parent {
            Some(parent) => tsubamedo_core::import::new_dir(&imported, Path::new(&parent)),
            None => self.state.lock().unwrap().collection()?.root.clone(),
        };
        let written = tsubamedo_core::import::write(&imported, &dir).map_err(|e| e.to_string())?;
        let info = self.open_collection(written.root.to_string_lossy().into_owned())?;
        Ok(json!({ "written": written, "collection": info }))
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
        let mut state = self.state.lock().unwrap();
        let captured: Vars =
            state.captured.get(env.unwrap_or_default()).map(|m| m.iter().map(|(k, v)| (k.clone(), v.value.clone())).collect()).unwrap_or_default();
        let jar = state.jar(env);
        let proxy = self.settings.lock().unwrap().proxy.clone();
        let options = SendOptions { verify_tls: !insecure, cookies: Some(jar), proxy, ..SendOptions::default() };
        let runner = Runner::new(state.collection.as_ref(), env, Vars::new(), captured, options).map_err(|e| e.to_string())?;
        Ok(runner.with_tokens(state.tokens.clone()))
    }

    /// Cookies and tokens to disk when they changed.
    fn save_stores(&self) {
        let Some(history) = self.history() else { return };
        let state = self.state.lock().unwrap();
        let mut cookies_changed = false;
        for jar in state.jars.values() {
            cookies_changed |= jar.lock().unwrap().take_changed();
        }
        if cookies_changed {
            let jars: BTreeMap<String, Jar> =
                state.jars.iter().map(|(env, jar)| (env.clone(), jar.lock().unwrap().clone())).filter(|(_, j)| !j.is_empty()).collect();
            let _ = history.save_cookies(&jars);
        }
        // Busy means a request is getting a token right now; it saves when it is done.
        if let Ok(mut tokens) = state.tokens.try_lock()
            && tokens.take_changed()
        {
            let _ = history.save_tokens(&tokens);
        }
    }

    /// The request's OAuth 2.0 settings with variables substituted, and the store they are in.
    fn oauth_settings(&self, a: &OAuthArg) -> Result<(OAuth2, SharedTokens), String> {
        let Auth::OAuth2(o) = &a.request.auth else { return Err("у запроса не OAuth 2.0".into()) };
        let runner = self.runner(a.env.as_deref(), false)?;
        let o = render_oauth(o, &runner.vars()).map_err(|e| e.to_string())?;
        Ok((o, self.state.lock().unwrap().tokens.clone()))
    }

    /// A response's status, headers and body: one in memory, or one from history.
    fn side(&self, s: SideArg) -> Result<Answer, String> {
        if let Some(id) = s.history_id {
            let history = self.history().ok_or("история не ведётся")?;
            let (entry, body) = history.get(&id).ok_or("этой записи в истории уже нет")?;
            let body = body.ok_or("у этой записи истории нет тела: ответы больше 1 МБ не сохраняются")?;
            let meta = entry.meta.unwrap_or(Value::Null);
            let headers: Vec<(String, String)> = serde_json::from_value(meta["headers"].clone()).unwrap_or_default();
            return Ok((entry.status.unwrap_or(0), headers, ResponseBody::Memory(body.into())));
        }
        let id = s.response_id.ok_or("не указан ответ")?;
        let body = self.state.lock().unwrap().body(id)?;
        Ok((s.status.unwrap_or(0), s.headers.unwrap_or_default(), body))
    }

    async fn compare(&self, a: CompareArg) -> Reply {
        let (left, right) = (self.side(a.left)?, self.side(a.right)?);
        let options = tsubamedo_core::compare::Options { ignore: a.ignore, all_headers: a.all_headers };
        blocking(move || {
            let (l, r) = (
                tsubamedo_core::compare::Side { status: left.0, headers: &left.1, body: &left.2 },
                tsubamedo_core::compare::Side { status: right.0, headers: &right.1, body: &right.2 },
            );
            let result = tsubamedo_core::compare::compare(&l, &r, &options).map_err(|e| e.to_string())?;
            reply(result)
        })
        .await
    }

    /// The collection's client certificates and whether each can be read in this environment.
    async fn certificates(&self, env: Option<String>) -> Reply {
        let (specs, vars, root) = {
            let state = self.state.lock().unwrap();
            let c = state.collection()?;
            let specs = c.certificates().map_err(|e| e.to_string())?;
            let vars = c.vars(env.as_deref(), &Vars::new()).map_err(|e| e.to_string())?;
            (specs, vars, c.root.clone())
        };
        // A PKCS#12 file is opened with `openssl`: off the async threads.
        blocking(move || {
            Ok(Value::Array(
                specs
                    .into_iter()
                    .map(|spec| {
                        let rendered = (|| {
                            let r = |s: &str| tsubamedo_core::template::render(s, &vars).map_err(|e| e.to_string());
                            Ok::<_, String>(CertSpec { host: r(&spec.host)?, cert: r(&spec.cert)?, key: r(&spec.key)?, passphrase: r(&spec.passphrase)? })
                        })();
                        let status = match rendered.map(|s| ClientCert::load(&s, &root).identity) {
                            Ok(Ok(identity)) => json!({ "ok": true, "text": format!("{}: сертификатов в цепочке {}", identity.label, identity.chain.len()) }),
                            Ok(Err(e)) | Err(e) => json!({ "ok": false, "text": e }),
                        };
                        json!({ "spec": spec, "status": status })
                    })
                    .collect(),
            ))
        })
        .await
    }

    /// The kept token for the request's settings, if any.
    async fn oauth_status(&self, a: OAuthArg) -> Reply {
        let (o, tokens) = match self.oauth_settings(&a) {
            Ok(found) => found,
            // Not all variables are set yet: there cannot be a token either.
            Err(_) => return Ok(Value::Null),
        };
        let tokens = tokens.lock().await;
        Ok(tokens.get(&o).map(token_info).unwrap_or(Value::Null))
    }

    /// A new token now, as the editor's «Получить токен» asks.
    async fn oauth_token(self: Arc<Self>, a: OAuthArg) -> Reply {
        let (o, tokens) = self.oauth_settings(&a)?;
        let proxy = self.settings.lock().unwrap().proxy.clone();
        let options = SendOptions { verify_tls: !a.insecure, proxy, ..SendOptions::default() };
        let task = tokio::spawn(async move { oauth::token(&o, &tokens, &options, oauth::Mode::New).await });
        let send_id = a.send_id.clone().unwrap_or_default();
        self.sends.lock().unwrap().insert(send_id.clone(), task.abort_handle());
        let result = task.await;
        self.sends.lock().unwrap().remove(&send_id);
        let got = match result {
            Ok(got) => got.map_err(|e| e.to_string())?,
            Err(e) if e.is_cancelled() => return Err("Отменено".into()),
            Err(e) => return Err(e.to_string()),
        };
        self.save_stores();
        Ok(token_info(&got.token))
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

    /// The request as code in `lang` (`curl` when not given).
    fn code(&self, a: CodeArg) -> Reply {
        let (_, dir) = self.context(a.id.as_deref(), a.env.as_deref())?;
        // Values captured from responses too, as a send would use.
        let mut vars = self.runner(a.env.as_deref(), false)?.vars();
        let mut prepared = match prepare_without_files(&a.request, &vars, &dir) {
            // Not set yet: a visible `<name>` in the code rather than no code.
            Err(tsubamedo_core::Error::UndefinedVars(names)) => {
                vars.extend(names.iter().map(|n| (n.clone(), format!("<{n}>"))));
                // In the address a placeholder makes no address: then the error as it was.
                prepare_without_files(&a.request, &vars, &dir).map_err(|_| tsubamedo_core::Error::UndefinedVars(names))
            }
            other => other,
        }
        .map_err(|e| e.to_string())?;
        // The kept OAuth 2.0 token, so the command works as is while it lasts.
        if let Some(o) = &prepared.oauth
            && prepared.header("authorization").is_none()
        {
            let tokens = self.state.lock().unwrap().tokens.clone();
            if let Ok(tokens) = tokens.try_lock()
                && let Some(token) = tokens.get(o)
            {
                prepared.headers.push(("Authorization".into(), token.header()));
            }
        }
        let spec = tsubamedo_core::codegen::Spec::new(&prepared, &a.request.body, &vars);
        Ok(Value::String(tsubamedo_core::codegen::generate(a.lang.as_deref().unwrap_or("curl"), &spec)?))
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
        self.save_stores();
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
    /// Runs requests of the collection; `runProgress` shows them as they go.
    async fn run_collection(self: Arc<Self>, a: RunArg) -> Reply {
        let mut runner = self.runner(a.env.as_deref(), a.insecure)?;
        let steps = {
            let state = self.state.lock().unwrap();
            let collection = state.collection()?;
            match &a.only {
                Some(ids) => runner.plan_only(collection, ids),
                None => {
                    let inside = |id: &str| a.folder.as_deref().is_none_or(|f| f.is_empty() || id.starts_with(&format!("{f}/")));
                    runner.plan(collection, |e| inside(&e.id))
                }
            }
        };
        let planned: Vec<Value> = steps
            .iter()
            .map(|s| json!({ "id": s.id, "method": s.request.method.to_ascii_uppercase(), "name": s.request.name }))
            .collect();
        let send_id = a.send_id.clone();
        self.runs.lock().unwrap().insert(send_id.clone(), json!({ "steps": planned, "running": {}, "results": [], "done": false }));
        let options = RunOptions { parallel: a.parallel.unwrap_or(1), retries: a.retries, ..RunOptions::default() };
        let started = std::time::Instant::now();
        let session = self.clone();
        let env = a.env.clone();
        let task = tokio::spawn(async move {
            runner
                .run(&steps, &options, |event| {
                    let mut runs = session.runs.lock().unwrap();
                    let Some(progress) = runs.get_mut(&send_id) else { return };
                    match event {
                        Event::Started { step, attempt } => progress["running"][step.to_string()] = json!(attempt),
                        Event::Finished { step, attempts, exchanges, .. } => {
                            if let Some(running) = progress["running"].as_object_mut() {
                                running.remove(&step.to_string());
                            }
                            let last = exchanges.len().saturating_sub(1);
                            for (i, ex) in exchanges.iter().enumerate() {
                                let mut result = run_result(ex);
                                // Requests sent first for missing values have no step of their own.
                                result["step"] = if i == last { json!(step) } else { Value::Null };
                                result["attempts"] = json!(if i == last { attempts } else { 1 });
                                if let Some(results) = progress["results"].as_array_mut() {
                                    results.push(result);
                                }
                            }
                            drop(runs);
                            session.keep_captured(env.as_deref(), &exchanges);
                            for ex in &exchanges {
                                session.record_exchange(ex, env.as_deref(), None);
                            }
                        }
                    }
                })
                .await;
        });
        self.sends.lock().unwrap().insert(a.send_id.clone(), task.abort_handle());
        let result = task.await;
        self.sends.lock().unwrap().remove(&a.send_id);
        self.save_stores();
        let progress = self.runs.lock().unwrap().remove(&a.send_id).unwrap_or(Value::Null);
        match result {
            Ok(()) => {}
            Err(e) if e.is_cancelled() => return Err("Отменено".into()),
            Err(e) => return Err(e.to_string()),
        }
        Ok(json!({ "steps": progress["steps"], "results": progress["results"], "totalMs": started.elapsed().as_secs_f64() * 1000.0 }))
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

/// A token as the editor shows it: never whole.
fn token_info(t: &oauth::Token) -> Value {
    let chars: Vec<char> = t.access_token.chars().collect();
    let preview = if chars.len() > 16 {
        format!("{}…{}", chars[..8].iter().collect::<String>(), chars[chars.len() - 4..].iter().collect::<String>())
    } else {
        "••••".to_string()
    };
    json!({
        "preview": preview,
        "obtainedAt": t.obtained_at,
        "expiresAt": t.expires_at,
        "hasRefresh": t.refresh_token.is_some(),
        "scope": t.scope,
    })
}

fn repo(root: &Path) -> Result<git::Repo, String> {
    git::Repo::open(root)?.ok_or_else(|| "коллекция не в git".to_string())
}

/// `{"git": "missing", "error"}` without git, `{"git": "none"}` outside a repository, else the
/// status with `"git": "repo"`.
fn git_status(root: &Path) -> Reply {
    match git::Repo::open(root) {
        Err(e) => Ok(json!({ "git": "missing", "error": e })),
        Ok(None) => Ok(json!({ "git": "none" })),
        Ok(Some(repo)) => {
            let mut status = serde_json::to_value(repo.status()?).map_err(|e| e.to_string())?;
            status["git"] = json!("repo");
            Ok(status)
        }
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
                name: e.request.name.clone().unwrap_or_else(|| e.id.rsplit('/').next().unwrap_or_default().to_string()),
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
struct CommitArg {
    paths: Vec<String>,
    message: String,
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

/// Status, headers and body of a response to compare.
type Answer = (u16, Vec<(String, String)>, ResponseBody);

/// A response to compare: `responseId` with what the window knows of it, or `historyId`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SideArg {
    response_id: Option<u64>,
    status: Option<u16>,
    headers: Option<Vec<(String, String)>>,
    history_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompareArg {
    left: SideArg,
    right: SideArg,
    #[serde(default)]
    ignore: Vec<String>,
    #[serde(default)]
    all_headers: bool,
}

#[derive(Deserialize)]
struct ProxyArg {
    proxy: ProxyConfig,
}

#[derive(Deserialize)]
struct CertificatesArg {
    env: Option<String>,
    certificates: Vec<CertSpec>,
}

#[derive(Deserialize)]
struct CookieArg {
    env: Option<String>,
    domain: String,
    path: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OAuthArg {
    request: Request,
    env: Option<String>,
    /// To cancel a sign-in that waits for the browser.
    send_id: Option<String>,
    #[serde(default)]
    insecure: bool,
}

/// A request of a run as the window lists it.
fn run_result(ex: &Exchange) -> Value {
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
        "logs": ex.logs,
    })
}

#[derive(Deserialize)]
struct ImportArg {
    path: String,
    /// A new collection in this folder; none means into the open one.
    parent: Option<String>,
}

#[derive(Deserialize)]
struct TextArg {
    text: String,
}

#[derive(Deserialize)]
struct CodeArg {
    id: Option<String>,
    request: Request,
    env: Option<String>,
    lang: Option<String>,
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
    /// Requests in flight at once; 1 when not given.
    parallel: Option<usize>,
    /// How many times a failed request is sent again.
    #[serde(default)]
    retries: u32,
    /// Only these requests (to send again what failed); `folder` is ignored then.
    only: Option<Vec<String>>,
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
    /// The proxy it went through.
    via: Option<String>,
    is_json: bool,
    content_type: Option<String>,
    /// Unix milliseconds.
    received_at: u64,
    sent: SentRequest,
    checks: Vec<CheckResult>,
    captures: Vec<Captured>,
    /// Requests sent first for the values this one needed.
    before: Vec<Before>,
    /// What the scripts wrote with `console.log`.
    logs: Vec<String>,
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
            via: r.via.clone(),
            is_json: r.is_json() || sniffed_json,
            content_type: r.header("content-type").map(str::to_string),
            received_at: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
            sent,
            checks: ex.checks.clone(),
            captures: ex.captures.clone(),
            before: Vec::new(),
            logs: ex.logs.clone(),
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
