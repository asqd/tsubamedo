//! The collection in git: what changed, commit, pull and push, through the system `git`: the
//! user's own, with their settings, keys and credential helpers. The app carries no git.
//!
//! A collection may be a folder of a bigger repository: only its files are shown and committed,
//! while pull and push move the whole branch, as git does.
//!
//! Nothing asks for a password: git runs without a terminal (`GIT_TERMINAL_PROMPT=0`), so a
//! remote that needs one fails with a message instead of waiting. Pull rebases the local
//! commits on top of the remote ones; when they conflict, the rebase is undone and the
//! repository stays as it was.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Secrets: never committed from here.
const SECRET: &str = ".local.toml";
const QUICK: Duration = Duration::from_secs(30);
const NETWORK: Duration = Duration::from_secs(120);

/// The repository a collection is in.
#[derive(Debug, Clone)]
pub struct Repo {
    git: PathBuf,
    /// The collection's folder.
    root: PathBuf,
    git_dir: PathBuf,
    /// The collection's folder inside the repository, `api/` or empty.
    prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// `None` on a detached HEAD.
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    /// No commit yet.
    pub unborn: bool,
    /// The collection's changes, paths from its folder.
    pub changes: Vec<Change>,
    /// Changes elsewhere in the repository: not shown and not committed from here.
    pub outside: usize,
    /// A merge or rebase left unfinished: finished in git, not here.
    pub busy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub path: String,
    pub kind: ChangeKind,
    /// The old path of a rename.
    pub from: Option<String>,
    /// A `*.local.toml`: secrets, never committed from here.
    pub secret: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Conflict,
}

/// The system git: on `PATH`, else where Homebrew and the system put it (an app started from
/// the Finder gets a short `PATH`). macOS's `/usr/bin/git` only counts with the developer
/// tools installed: without them it opens an installer instead of running.
fn git() -> Result<PathBuf, String> {
    static FOUND: OnceLock<PathBuf> = OnceLock::new();
    if let Some(found) = FOUND.get() {
        return Ok(found.clone());
    }
    let name = if cfg!(windows) { "git.exe" } else { "git" };
    let on_path = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).map(|d| d.join(name)).collect::<Vec<_>>()).unwrap_or_default();
    let known = ["/opt/homebrew/bin/git", "/usr/local/bin/git", "/usr/bin/git"].map(PathBuf::from);
    let found = on_path.into_iter().chain(known).find(|p| {
        p.is_file() && (!cfg!(target_os = "macos") || p != Path::new("/usr/bin/git") || Command::new("xcode-select").arg("-p").output().is_ok_and(|o| o.status.success()))
    });
    match found {
        Some(path) => Ok(FOUND.get_or_init(|| path).clone()),
        None if cfg!(target_os = "macos") => Err(tr!(
            "git не найден: установите его — xcode-select --install или brew install git",
            "git not found: install it — xcode-select --install or brew install git"
        )),
        None => Err(tr!("git не найден: установите его", "git not found: install it")),
    }
}

/// One change to repositories at a time: two fetches at once (the window coming back while the
/// panel opens) fail on git's own lock of the remote's refs.
fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs git in `dir`; stdout, or what went wrong in words.
fn run(git: &Path, dir: &Path, args: &[&str], stdin: Option<&str>, limit: Duration) -> Result<String, String> {
    let mut child = Command::new(git)
        .args(["-c", "core.quotepath=false", "-c", "color.ui=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| tr!("git не запустился: {e}", "git did not start: {e}"))?;
    if let (Some(text), Some(mut pipe)) = (stdin, child.stdin.take()) {
        use std::io::Write;
        pipe.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut out);
            }
            String::from_utf8_lossy(&out).into_owned()
        })
    };
    let out = read(child.stdout.take().map(|p| Box::new(p) as _));
    let err = read(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(tr!("git {}: нет ответа за {} с", "git {}: no answer in {} s", args[0], limit.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(15));
    };
    let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    if status.success() { Ok(out) } else { Err(explain(&err, &out)) }
}

/// git's complaint in words, for the ones a user meets; the rest as git said it.
fn explain(err: &str, out: &str) -> String {
    let has = |s: &str| err.contains(s);
    if has("Please tell me who you are") || has("unable to auto-detect email address") {
        return tr!(
            "git не знает автора: git config --global user.name \"Имя\" и git config --global user.email адрес",
            "git does not know the author: git config --global user.name \"Name\" and git config --global user.email address"
        );
    }
    if has("terminal prompts disabled") || has("could not read Username") || has("Authentication failed") {
        return tr!(
            "сервер просит войти: настройте ключ SSH или хранилище паролей git — в терминале достаточно войти один раз",
            "the server asks to sign in: set up an SSH key or a git credential helper — signing in once in a terminal is enough"
        );
    }
    if has("Permission denied (publickey") || has("Host key verification failed") {
        return tr!(
            "ключ SSH не подошёл: проверьте, что он добавлен на сервер и в ssh-agent (ssh-add)",
            "the SSH key was not accepted: check that it is added on the server and to ssh-agent (ssh-add)"
        );
    }
    if let Some(host) = err.split("Could not resolve host: ").nth(1) {
        return tr!("нет связи с {}", "cannot reach {}", host.lines().next().unwrap_or_default().trim());
    }
    if has("[rejected]") || has("non-fast-forward") || has("fetch first") {
        return tr!("на сервере есть коммиты, которых здесь нет: сначала «Получить»", "the server has commits that are not here: Pull first");
    }
    let text = if err.trim().is_empty() { out } else { err };
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("hint:")).collect();
    let said = lines.join("; ");
    said.strip_prefix("fatal: ").or_else(|| said.strip_prefix("error: ")).unwrap_or(&said).to_string()
}

impl Repo {
    /// The repository the collection is in; `None` when it is in none.
    pub fn open(root: &Path) -> Result<Option<Repo>, String> {
        let git = git()?;
        match run(&git, root, &["rev-parse", "--show-prefix", "--absolute-git-dir"], None, QUICK) {
            Ok(out) => {
                let mut lines = out.lines();
                let prefix = lines.next().unwrap_or_default().to_string();
                let git_dir = PathBuf::from(lines.next().unwrap_or_default());
                Ok(Some(Repo { git, root: root.to_path_buf(), git_dir, prefix }))
            }
            Err(e) if e.contains("not a git repository") => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A new repository in the collection's folder, secrets left out of it.
    pub fn init(root: &Path) -> Result<Repo, String> {
        run(&git()?, root, &["init", "--quiet"], None, QUICK)?;
        let repo = Repo::open(root)?.ok_or(tr_static!("git init не создал репозиторий", "git init did not create a repository"))?;
        repo.ignore_secrets()?;
        Ok(repo)
    }

    pub fn status(&self) -> Result<Status, String> {
        let out = self.git(&["status", "--porcelain=v2", "-z", "--branch", "--untracked-files=all"], QUICK)?;
        let mut status = Status {
            branch: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            unborn: false,
            changes: Vec::new(),
            outside: 0,
            busy: self.busy(),
        };
        let mut entries = out.split('\0').filter(|e| !e.is_empty());
        while let Some(entry) = entries.next() {
            let (kind, path, from) = match entry.as_bytes()[0] {
                b'#' => {
                    let mut words = entry[2..].splitn(2, ' ');
                    let (key, value) = (words.next().unwrap_or_default(), words.next().unwrap_or_default());
                    match key {
                        "branch.oid" => status.unborn = value == "(initial)",
                        "branch.head" if value != "(detached)" => status.branch = Some(value.to_string()),
                        "branch.upstream" => status.upstream = Some(value.to_string()),
                        "branch.ab" => {
                            let mut ab = value.split(' ').map(|n| n.trim_start_matches(['+', '-']).parse().unwrap_or(0));
                            status.ahead = ab.next().unwrap_or(0);
                            status.behind = ab.next().unwrap_or(0);
                        }
                        _ => {}
                    }
                    continue;
                }
                b'1' => (kind(&entry[2..4]), field(entry, 8), None),
                // A rename or copy: the old path is the next entry.
                b'2' => (ChangeKind::Renamed, field(entry, 9), entries.next()),
                b'u' => (ChangeKind::Conflict, field(entry, 10), None),
                b'?' => (ChangeKind::Added, &entry[2..], None),
                _ => continue,
            };
            match path.strip_prefix(self.prefix.as_str()) {
                Some(path) => status.changes.push(Change {
                    path: path.to_string(),
                    kind,
                    from: from.map(|f| f.strip_prefix(self.prefix.as_str()).unwrap_or(f).to_string()),
                    secret: path.ends_with(SECRET),
                }),
                None => status.outside += 1,
            }
        }
        status.changes.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(status)
    }

    /// What changed in a file since the last commit, as a unified diff; a new file whole.
    pub fn diff(&self, path: &str) -> Result<String, String> {
        let full = self.full(path);
        if self.git(&["rev-parse", "--verify", "--quiet", "HEAD"], QUICK).is_ok() {
            let diff = self.git(&["diff", "--no-ext-diff", "HEAD", "--", &full], QUICK)?;
            if !diff.is_empty() {
                return Ok(diff);
            }
        }
        if !self.root.join(path).is_file() {
            return Ok(String::new());
        }
        // `--no-index` answers 1 when the files differ, which they always do here.
        match run(&self.git, &self.root, &["diff", "--no-ext-diff", "--no-index", "--", null(), path], None, QUICK) {
            Ok(diff) => Ok(diff),
            Err(diff) if diff.starts_with("diff --git") => Ok(diff),
            Err(e) => Err(e),
        }
    }

    /// Commits these files of the collection (paths from its folder; a rename needs both of its
    /// paths) and nothing else, whatever else is staged. Returns the short hash.
    pub fn commit(&self, paths: &[String], message: &str) -> Result<String, String> {
        if message.trim().is_empty() {
            return Err(tr!("нужно описание коммита", "the commit needs a message"));
        }
        if paths.is_empty() {
            return Err(tr!("не выбрано ни одного файла", "no files selected"));
        }
        if let Some(secret) = paths.iter().find(|p| p.ends_with(SECRET)) {
            return Err(tr!("{secret}: в *.local.toml секреты, они не коммитятся", "{secret}: *.local.toml files hold secrets, they are not committed"));
        }
        let _one = exclusive();
        if let Some(busy) = self.busy() {
            return Err(tr!("в репозитории не закончено {busy} — закончите его в git", "a {busy} is in progress in the repository — finish it in git"));
        }
        let full: Vec<String> = paths.iter().map(|p| self.full(p)).collect();
        // What is on disk is added; what is gone is removed, even when the index has already
        // forgotten it (the old path of a staged rename), which `git add` would refuse.
        let (present, gone): (Vec<_>, Vec<_>) = paths.iter().zip(&full).partition(|(p, _)| self.root.join(p).exists());
        if !present.is_empty() {
            let mut add = vec!["add", "--all", "--"];
            add.extend(present.iter().map(|(_, f)| f.as_str()));
            self.git(&add, QUICK)?;
        }
        if !gone.is_empty() {
            let mut rm = vec!["rm", "--cached", "--quiet", "--ignore-unmatch", "--"];
            rm.extend(gone.iter().map(|(_, f)| f.as_str()));
            self.git(&rm, QUICK)?;
        }
        let mut commit = vec!["commit", "--quiet", "--file=-", "--"];
        commit.extend(full.iter().map(String::as_str));
        run(&self.git, &self.root, &commit, Some(message.trim()), QUICK)?;
        Ok(self.git(&["rev-parse", "--short", "HEAD"], QUICK)?.trim().to_string())
    }

    /// What the remote has, for the counts; quietly, since it is only a look. Without a remote
    /// there is nothing to look at.
    pub fn fetch(&self) -> Result<(), String> {
        let _one = exclusive();
        if self.git(&["remote"], QUICK)?.trim().is_empty() {
            return Ok(());
        }
        self.git(&["fetch", "--quiet"], NETWORK).map(|_| ())
    }

    /// The remote's commits, with the local ones put on top of them. Says how many came.
    pub fn pull(&self) -> Result<String, String> {
        let _one = exclusive();
        if let Some(busy) = self.busy() {
            return Err(tr!("в репозитории не закончено {busy} — закончите его в git", "a {busy} is in progress in the repository — finish it in git"));
        }
        let status = self.status()?;
        let branch = status.branch.ok_or(tr_static!("HEAD не на ветке: переключитесь на ветку в git", "HEAD is not on a branch: switch to a branch in git"))?;
        if status.upstream.is_none() {
            return Err(tr!("ветка {branch} ещё не связана с сервером — сначала «Отправить»", "branch {branch} is not tied to the server yet — Push first"));
        }
        let before = self.git(&["rev-parse", "HEAD"], QUICK)?.trim().to_string();
        if let Err(e) = self.git(&["pull", "--rebase", "--autostash", "--quiet"], NETWORK) {
            if self.busy().is_some() {
                let files = self.git(&["diff", "--name-only", "--diff-filter=U"], QUICK).unwrap_or_default();
                let _ = self.git(&["rebase", "--abort"], QUICK);
                let files: Vec<&str> = files.lines().map(|f| f.strip_prefix(self.prefix.as_str()).unwrap_or(f)).collect();
                return Err(tr!(
                    "на сервере те же места изменены иначе: {} — ничего не тронуто, разберите конфликт в git",
                    "the server changed the same places differently: {} — nothing was touched, resolve the conflict in git",
                    files.join(", ")
                ));
            }
            return Err(e);
        }
        let came: u32 = self.git(&["rev-list", "--count", &format!("{before}..@{{upstream}}")], QUICK)?.trim().parse().unwrap_or(0);
        Ok(match came {
            0 => tr!("нового на сервере нет", "nothing new on the server"),
            n => tr!("получено коммитов: {n}", "commits pulled: {n}"),
        })
    }

    /// The branch's commits to the remote; a new branch goes to `origin` (or the only remote)
    /// and is tied to it.
    pub fn push(&self) -> Result<String, String> {
        let _one = exclusive();
        let status = self.status()?;
        let branch = status.branch.ok_or(tr_static!("HEAD не на ветке: переключитесь на ветку в git", "HEAD is not on a branch: switch to a branch in git"))?;
        if status.upstream.is_some() {
            if status.ahead == 0 {
                return Ok(tr!("отправлять нечего", "nothing to push"));
            }
            self.git(&["push", "--quiet"], NETWORK)?;
            return Ok(tr!("отправлено коммитов: {}", "commits pushed: {}", status.ahead));
        }
        let remotes = self.git(&["remote"], QUICK)?;
        let remotes: Vec<&str> = remotes.lines().collect();
        let remote = match remotes.as_slice() {
            [] => return Err(tr!("у репозитория нет сервера: git remote add origin <адрес>", "the repository has no server: git remote add origin <address>")),
            [one] => *one,
            many if many.contains(&"origin") => "origin",
            many => return Err(tr!(
                    "серверов несколько ({}), а origin среди них нет: отправьте ветку в git",
                    "there are several servers ({}) and none is origin: push the branch in git",
                    many.join(", ")
                )),
        };
        self.git(&["push", "--quiet", "--set-upstream", remote, &branch], NETWORK)?;
        Ok(tr!("ветка {branch} отправлена в {remote}", "branch {branch} pushed to {remote}"))
    }

    /// `*.local.toml` into the collection's `.gitignore`.
    pub fn ignore_secrets(&self) -> Result<(), String> {
        let path = self.root.join(".gitignore");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        if text.lines().any(|l| l.trim() == "*.local.toml") {
            return Ok(());
        }
        let sep = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
        std::fs::write(&path, format!("{text}{sep}*.local.toml\n")).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn git(&self, args: &[&str], limit: Duration) -> Result<String, String> {
        run(&self.git, &self.root, args, None, limit)
    }

    /// A collection path as git names it, from the top of the repository. git takes it from
    /// any folder of the repository with `:/`.
    fn full(&self, path: &str) -> String {
        format!(":/{}{path}", self.prefix)
    }

    fn busy(&self) -> Option<String> {
        let at = |name: &str| self.git_dir.join(name).exists();
        if at("rebase-merge") || at("rebase-apply") {
            Some(tr!("перебазирование (rebase)", "rebase"))
        } else if at("MERGE_HEAD") {
            Some(tr!("слияние (merge)", "merge"))
        } else if at("CHERRY_PICK_HEAD") {
            Some("cherry-pick".into())
        } else {
            None
        }
    }
}

/// The path after `n` space-separated fields of a status line.
fn field(entry: &str, n: usize) -> &str {
    entry.splitn(n + 1, ' ').nth(n).unwrap_or_default()
}

fn kind(xy: &str) -> ChangeKind {
    match xy.as_bytes() {
        [b'D', _] | [_, b'D'] => ChangeKind::Deleted,
        [b'A', _] => ChangeKind::Added,
        [b'R' | b'C', _] => ChangeKind::Renamed,
        _ => ChangeKind::Modified,
    }
}

fn null() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complaints_in_words() {
        assert!(explain("*** Please tell me who you are.\n\nRun\n", "").starts_with("git не знает автора"));
        assert!(explain("fatal: could not read Username for 'https://github.com': terminal prompts disabled\n", "").starts_with("сервер просит войти"));
        assert!(explain("git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.\n", "").starts_with("ключ SSH не подошёл"));
        assert_eq!(explain("fatal: unable to access 'https://x.test/r.git/': Could not resolve host: x.test\n", ""), "нет связи с x.test");
        assert_eq!(explain("hint: see the docs\nfatal: something odd\n", ""), "something odd");
        crate::lang::in_language(true, || {
            assert!(explain("*** Please tell me who you are.\n\nRun\n", "").starts_with("git does not know the author"));
            assert_eq!(explain("fatal: unable to access 'https://x.test/r.git/': Could not resolve host: x.test\n", ""), "cannot reach x.test");
            assert_eq!(explain(" ! [rejected] main -> main (fetch first)\n", ""), "the server has commits that are not here: Pull first");
        });
    }

    #[test]
    fn status_lines() {
        assert_eq!(field("1 .M N... 100644 100644 100644 abc abc api/a b.toml", 8), "api/a b.toml");
        assert_eq!(kind(".D"), ChangeKind::Deleted);
        assert_eq!(kind("A."), ChangeKind::Added);
        assert_eq!(kind("MM"), ChangeKind::Modified);
    }
}
