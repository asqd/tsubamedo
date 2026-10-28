//! The collection's git on real repositories: a bare one as the server and clones of it, with
//! the collection in a folder `api/` of each.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use tsubamedo_core::git::{Change, ChangeKind, Repo};

/// The same git settings for every test, whatever the computer's are.
fn isolate() {
    static CONFIG: OnceLock<PathBuf> = OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        let path = dir.join("gitconfig");
        fs::write(&path, "[user]\n\tname = Кот\n\temail = kot@test.ru\n[init]\n\tdefaultBranch = main\n[pull]\n\trebase = false\n").unwrap();
        path
    });
    // SAFETY: every test sets the same values before running git.
    unsafe {
        std::env::set_var("GIT_CONFIG_GLOBAL", config);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn write(dir: &Path, path: &str, text: &str) {
    let path = dir.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A server with one commit and a clone of it: `(temp, server, clone)`.
fn server_and_clone() -> (tempfile::TempDir, PathBuf, PathBuf) {
    isolate();
    let temp = tempfile::tempdir().unwrap();
    let server = temp.path().join("server.git");
    git(temp.path(), &["init", "--quiet", "--bare", "server.git"]);
    let a = temp.path().join("a");
    git(temp.path(), &["clone", "--quiet", "server.git", "a"]);
    write(&a, "README.md", "репозиторий\n");
    write(&a, "api/tsubamedo.toml", "name = \"Магазин\"\n");
    write(&a, "api/orders/list.toml", "name = \"Товары\"\nurl = \"{{base}}/items\"\n");
    write(&a, "api/old.toml", "url = \"/old\"\n");
    write(&a, "api/a.toml", "url = \"/a\"\n");
    git(&a, &["add", "."]);
    git(&a, &["commit", "--quiet", "-m", "начало"]);
    git(&a, &["push", "--quiet", "-u", "origin", "main"]);
    (temp, server, a)
}

fn paths(changes: &[Change]) -> Vec<(&str, ChangeKind)> {
    changes.iter().map(|c| (c.path.as_str(), c.kind)).collect()
}

#[test]
fn status_and_commit_of_a_collection_in_a_folder() {
    let (_temp, _, a) = server_and_clone();
    write(&a, "api/orders/list.toml", "name = \"Все товары\"\nurl = \"{{base}}/items\"\n");
    fs::remove_file(a.join("api/old.toml")).unwrap();
    write(&a, "api/new.toml", "url = \"/new\"\n");
    write(&a, "api/tsubamedo.local.toml", "[vars]\ntoken = \"секрет\"\n");
    write(&a, "README.md", "репозиторий с API\n");
    git(&a, &["mv", "api/a.toml", "api/b.toml"]);

    let repo = Repo::open(&a.join("api")).unwrap().unwrap();
    let status = repo.status().unwrap();
    assert_eq!((status.branch.as_deref(), status.upstream.as_deref(), status.ahead, status.behind), (Some("main"), Some("origin/main"), 0, 0));
    assert_eq!(
        paths(&status.changes),
        [
            ("b.toml", ChangeKind::Renamed),
            ("new.toml", ChangeKind::Added),
            ("old.toml", ChangeKind::Deleted),
            ("orders/list.toml", ChangeKind::Modified),
            ("tsubamedo.local.toml", ChangeKind::Added),
        ]
    );
    assert_eq!(status.changes[0].from.as_deref(), Some("a.toml"));
    assert!(status.changes[4].secret && !status.changes[3].secret);
    assert_eq!(status.outside, 1, "README вне коллекции");

    let diff = repo.diff("orders/list.toml").unwrap();
    assert!(diff.contains("-name = \"Товары\"\n+name = \"Все товары\""), "{diff}");
    assert!(repo.diff("new.toml").unwrap().contains("+url = \"/new\""));
    assert!(repo.diff("old.toml").unwrap().contains("-url = \"/old\""));

    // Only what was chosen goes, the rename with both of its paths.
    repo.commit(&["orders/list.toml".into(), "new.toml".into()], "Товары: новое имя").unwrap();
    repo.commit(&["b.toml".into(), "a.toml".into(), "old.toml".into()], "переименование").unwrap();
    let status = repo.status().unwrap();
    assert_eq!(paths(&status.changes), [("tsubamedo.local.toml", ChangeKind::Added)]);
    assert_eq!((status.ahead, status.outside), (2, 1));
    assert_eq!(git(&a, &["log", "-1", "--format=%s", "--name-status"]).trim(), "переименование\n\nR100\tapi/a.toml\tapi/b.toml\nD\tapi/old.toml");

    assert!(repo.commit(&["tsubamedo.local.toml".into()], "секреты").unwrap_err().contains("секреты"));
    assert_eq!(repo.commit(&["README.md".into()], " ").unwrap_err(), "нужно описание коммита");
    repo.ignore_secrets().unwrap();
    repo.ignore_secrets().unwrap();
    assert_eq!(fs::read_to_string(a.join("api/.gitignore")).unwrap(), "*.local.toml\n");
    assert_eq!(paths(&repo.status().unwrap().changes), [(".gitignore", ChangeKind::Added)]);
}

#[test]
fn push_pull_and_a_conflict_that_changes_nothing() {
    let (temp, _, a) = server_and_clone();
    git(temp.path(), &["clone", "--quiet", "server.git", "b"]);
    let b = temp.path().join("b");
    let (ra, rb) = (Repo::open(&a.join("api")).unwrap().unwrap(), Repo::open(&b.join("api")).unwrap().unwrap());

    // A changes the list, B something else: B's pull puts its commit on top of A's.
    write(&a, "api/orders/list.toml", "name = \"Товары A\"\nurl = \"{{base}}/items\"\n");
    ra.commit(&["orders/list.toml".into()], "A").unwrap();
    assert_eq!(ra.push().unwrap(), "отправлено коммитов: 1");
    assert_eq!(ra.push().unwrap(), "отправлять нечего");
    write(&b, "api/new.toml", "url = \"/b\"\n");
    rb.commit(&["new.toml".into()], "B").unwrap();
    assert_eq!(rb.push().unwrap_err(), "на сервере есть коммиты, которых здесь нет: сначала «Получить»");
    rb.fetch().unwrap();
    let status = rb.status().unwrap();
    assert_eq!((status.ahead, status.behind), (1, 1));
    assert_eq!(rb.pull().unwrap(), "получено коммитов: 1");
    assert_eq!(rb.pull().unwrap(), "нового на сервере нет");
    assert_eq!(git(&b, &["log", "--format=%s"]).trim(), "B\nA\nначало");
    rb.push().unwrap();

    // Both change the same line: nothing is touched, B's commit and an edit in progress stay.
    ra.pull().unwrap();
    write(&a, "api/orders/list.toml", "name = \"Товары A2\"\nurl = \"{{base}}/items\"\n");
    ra.commit(&["orders/list.toml".into()], "A2").unwrap();
    ra.push().unwrap();
    write(&b, "api/orders/list.toml", "name = \"Товары B2\"\nurl = \"{{base}}/items\"\n");
    rb.commit(&["orders/list.toml".into()], "B2").unwrap();
    write(&b, "api/new.toml", "url = \"/b, правка\"\n");
    let head = git(&b, &["rev-parse", "HEAD"]);
    let e = rb.pull().unwrap_err();
    assert!(e.contains("orders/list.toml") && e.contains("ничего не тронуто"), "{e}");
    let status = rb.status().unwrap();
    assert_eq!((status.busy, git(&b, &["rev-parse", "HEAD"])), (None, head));
    assert_eq!(fs::read_to_string(b.join("api/new.toml")).unwrap(), "url = \"/b, правка\"\n");
    assert_eq!(paths(&status.changes), [("new.toml", ChangeKind::Modified)]);
}

#[test]
fn a_new_branch_goes_to_origin_and_stays_tied() {
    let (_temp, _, a) = server_and_clone();
    git(&a, &["checkout", "--quiet", "-b", "feature"]);
    let repo = Repo::open(&a.join("api")).unwrap().unwrap();
    assert_eq!(repo.pull().unwrap_err(), "ветка feature ещё не связана с сервером — сначала «Отправить»");
    write(&a, "api/new.toml", "url = \"/new\"\n");
    repo.commit(&["new.toml".into()], "новое").unwrap();
    assert_eq!(repo.push().unwrap(), "ветка feature отправлена в origin");
    assert_eq!(repo.status().unwrap().upstream.as_deref(), Some("origin/feature"));
}

#[test]
fn a_folder_without_git_and_a_new_repository() {
    isolate();
    let temp = tempfile::tempdir().unwrap();
    assert!(Repo::open(temp.path()).unwrap().is_none());
    write(temp.path(), "tsubamedo.toml", "name = \"Новая\"\n");
    write(temp.path(), "tsubamedo.local.toml", "[vars]\n");
    let repo = Repo::init(temp.path()).unwrap();
    let status = repo.status().unwrap();
    assert!(status.unborn && status.upstream.is_none());
    assert_eq!(paths(&status.changes), [(".gitignore", ChangeKind::Added), ("tsubamedo.toml", ChangeKind::Added)]);
    assert!(repo.diff("tsubamedo.toml").unwrap().contains("+name = \"Новая\""));
    repo.commit(&[".gitignore".into(), "tsubamedo.toml".into()], "коллекция").unwrap();
    assert!(repo.status().unwrap().changes.is_empty() && !repo.status().unwrap().unborn);
    repo.fetch().unwrap();
    assert_eq!(repo.push().unwrap_err(), "у репозитория нет сервера: git remote add origin <адрес>");
}
