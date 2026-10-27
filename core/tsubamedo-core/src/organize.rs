//! Renaming, moving, copying and deleting requests and folders of a collection. A request's
//! name is also its file name, so the folder reads the same as the sidebar. Deleted files go
//! to the system Trash, never straight away.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::collection::MANIFEST;
use crate::request_file::RequestFile;
use crate::{Error, Result};

/// A name usable as a file name on every system: no separators or reserved characters, no
/// leading dot (hidden), no `.local` ending (those files hold secrets and are skipped).
pub fn clean_name(name: &str) -> Result<String> {
    let cleaned: String = name.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '-' } else { c }).collect();
    let mut cleaned = cleaned.trim().trim_start_matches('.').trim().to_string();
    while let Some(rest) = cleaned.strip_suffix(".local") {
        cleaned = rest.trim_end().to_string();
    }
    if let Some(rest) = cleaned.strip_suffix(".toml") {
        cleaned = rest.trim_end().to_string();
    }
    if cleaned.is_empty() {
        return Err(Error::Invalid("имя не может быть пустым".into()));
    }
    Ok(cleaned)
}

/// The path of a collection-relative id, refusing anything that would leave the collection.
fn inside(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(Error::Invalid(format!("путь «{rel}» выходит за пределы коллекции")));
    }
    Ok(root.join(rel_path))
}

fn request_path(root: &Path, id: &str) -> Result<PathBuf> {
    let path = inside(root, &format!("{id}.toml"))?;
    if !path.is_file() {
        return Err(Error::Invalid(format!("запроса {id} нет в коллекции")));
    }
    Ok(path)
}

fn folder_path(root: &Path, folder: &str) -> Result<PathBuf> {
    if folder.trim_matches('/').is_empty() {
        return Ok(root.to_path_buf());
    }
    inside(root, folder.trim_matches('/'))
}

fn id_of(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path).with_extension("");
    rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

fn folder_id(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// `dir/base.toml`, or `dir/base 2.toml` and so on when taken. `keep` is the path being renamed:
/// it may be reused, so changing only the case works on case-insensitive disks.
fn free_path(dir: &Path, base: &str, ext: &str, keep: Option<&Path>) -> PathBuf {
    // Names the collection itself uses at its top: the manifest and the environments folder.
    let top = dir.join(MANIFEST).is_file();
    let reserved = |name: &str| top && (name.to_lowercase() == MANIFEST || (ext.is_empty() && name == "env"));
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{base}{ext}") } else { format!("{base} {n}{ext}") };
        let path = dir.join(&name);
        let same = keep.is_some_and(|k| {
            k.parent() == Some(dir) && k.file_name().is_some_and(|f| f.to_string_lossy().to_lowercase() == name.to_lowercase())
        });
        if same || (!path.exists() && !reserved(&name)) {
            return path;
        }
        n += 1;
    }
}

fn set_name(path: &Path, name: Option<String>) -> Result<()> {
    let mut file = RequestFile::open(path)?;
    let mut request = file.request()?;
    request.name = name;
    file.update(&request);
    file.save()?;
    Ok(())
}

/// New file name and `name` field. Returns the new id.
pub fn rename_request(root: &Path, id: &str, name: &str) -> Result<String> {
    let path = request_path(root, id)?;
    let base = clean_name(name)?;
    let dir = path.parent().unwrap_or(root);
    let dest = free_path(dir, &base, ".toml", Some(&path));
    if dest != path {
        fs::rename(&path, &dest).map_err(|e| Error::io(&path, e))?;
    }
    set_name(&dest, Some(name.trim().to_string()))?;
    Ok(id_of(root, &dest))
}

/// Into another folder (`""` is the collection root), created when missing.
pub fn move_request(root: &Path, id: &str, folder: &str) -> Result<String> {
    let path = request_path(root, id)?;
    let dir = folder_path(root, folder)?;
    if path.parent() == Some(dir.as_path()) {
        return Ok(id.to_string());
    }
    fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let dest = free_path(&dir, &stem, ".toml", None);
    fs::rename(&path, &dest).map_err(|e| Error::io(&path, e))?;
    Ok(id_of(root, &dest))
}

pub fn duplicate_request(root: &Path, id: &str) -> Result<String> {
    let path = request_path(root, id)?;
    let request = RequestFile::open(&path)?.request()?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let dest = free_path(path.parent().unwrap_or(root), &format!("{stem} копия"), ".toml", None);
    fs::copy(&path, &dest).map_err(|e| Error::io(&dest, e))?;
    let name = request.name.map(|n| format!("{n} (копия)"));
    if name.is_some() {
        set_name(&dest, name)?;
    }
    Ok(id_of(root, &dest))
}

pub fn delete_request(root: &Path, id: &str) -> Result<()> {
    to_trash(&request_path(root, id)?)
}

/// Returns the new folder's id.
pub fn create_folder(root: &Path, parent: &str, name: &str) -> Result<String> {
    let dir = folder_path(root, parent)?;
    let dest = free_path(&dir, &clean_name(name)?, "", None);
    fs::create_dir_all(&dest).map_err(|e| Error::io(&dest, e))?;
    Ok(folder_id(root, &dest))
}

fn existing_folder(root: &Path, folder: &str) -> Result<PathBuf> {
    let path = folder_path(root, folder)?;
    if path == root || !path.is_dir() {
        return Err(Error::Invalid(format!("папки «{folder}» нет в коллекции")));
    }
    Ok(path)
}

pub fn rename_folder(root: &Path, folder: &str, name: &str) -> Result<String> {
    let path = existing_folder(root, folder)?;
    let parent = path.parent().unwrap_or(root);
    let dest = free_path(parent, &clean_name(name)?, "", Some(&path));
    if dest != path {
        fs::rename(&path, &dest).map_err(|e| Error::io(&path, e))?;
    }
    Ok(folder_id(root, &dest))
}

/// Into another folder; a folder cannot go inside itself.
pub fn move_folder(root: &Path, folder: &str, parent: &str) -> Result<String> {
    let path = existing_folder(root, folder)?;
    let dir = folder_path(root, parent)?;
    if dir.starts_with(&path) {
        return Err(Error::Invalid("папку нельзя перенести в неё саму".into()));
    }
    if path.parent() == Some(dir.as_path()) {
        return Ok(folder.to_string());
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dest = free_path(&dir, &name, "", None);
    fs::rename(&path, &dest).map_err(|e| Error::io(&path, e))?;
    Ok(folder_id(root, &dest))
}

/// With everything in it.
pub fn delete_folder(root: &Path, folder: &str) -> Result<()> {
    to_trash(&existing_folder(root, folder)?)
}

#[cfg(not(test))]
fn to_trash(path: &Path) -> Result<()> {
    trash::delete(path).map_err(|e| Error::Invalid(format!("не удалось переместить {} в Корзину: {e}", path.display())))
}

// Tests must not fill the developer's Trash.
#[cfg(test)]
fn to_trash(path: &Path) -> Result<()> {
    if path.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) }.map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collection::Collection;

    fn collection() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(MANIFEST), "name = \"t\"\n").unwrap();
        fs::create_dir_all(dir.path().join("orders")).unwrap();
        fs::create_dir_all(dir.path().join("env")).unwrap();
        fs::write(dir.path().join("orders/create.toml"), "# мой комментарий\nname = \"Создать\"\nmethod = \"POST\"\nurl = \"{{base}}/orders\"\n").unwrap();
        dir
    }

    fn ids(root: &Path) -> Vec<String> {
        Collection::open(root).unwrap().requests.into_iter().map(|e| e.id).collect()
    }

    #[test]
    fn rename_moves_the_file_and_keeps_the_rest() {
        let dir = collection();
        let root = dir.path();
        let id = rename_request(root, "orders/create", "Создать заказ: v2").unwrap();
        assert_eq!(id, "orders/Создать заказ- v2");
        let text = fs::read_to_string(root.join("orders/Создать заказ- v2.toml")).unwrap();
        assert_eq!(text, "# мой комментарий\nname = \"Создать заказ: v2\"\nmethod = \"POST\"\nurl = \"{{base}}/orders\"\n");
        assert_eq!(rename_request(root, &id, "создать заказ- V2").unwrap(), "orders/создать заказ- V2");
        assert!(rename_request(root, "orders/nope", "x").is_err());
        assert!(rename_request(root, "../escape", "x").is_err());
        assert!(rename_request(root, "orders/создать заказ- V2", " . ").is_err());
    }

    #[test]
    fn move_copy_and_delete() {
        let dir = collection();
        let root = dir.path();
        let copy = duplicate_request(root, "orders/create").unwrap();
        assert_eq!(copy, "orders/create копия");
        assert!(fs::read_to_string(root.join("orders/create копия.toml")).unwrap().contains("name = \"Создать (копия)\""));
        let moved = move_request(root, &copy, "archive/2026").unwrap();
        assert_eq!(moved, "archive/2026/create копия");
        let back = move_request(root, &moved, "orders").unwrap();
        assert_eq!(back, "orders/create копия");
        assert_eq!(move_request(root, "orders/create", "").unwrap(), "create");
        assert_eq!(ids(root), ["create", "orders/create копия"]);
        delete_request(root, "create").unwrap();
        assert_eq!(ids(root), ["orders/create копия"]);
    }

    #[test]
    fn folders() {
        let dir = collection();
        let root = dir.path();
        assert_eq!(create_folder(root, "", "Туры").unwrap(), "Туры");
        assert_eq!(create_folder(root, "", "Туры").unwrap(), "Туры 2");
        assert_eq!(create_folder(root, "", "env").unwrap(), "env 2");
        assert_eq!(create_folder(root, "Туры", "Поиск").unwrap(), "Туры/Поиск");
        assert_eq!(rename_folder(root, "orders", "Заказы").unwrap(), "Заказы");
        assert_eq!(ids(root), ["Заказы/create"]);
        assert_eq!(move_folder(root, "Заказы", "Туры").unwrap(), "Туры/Заказы");
        assert!(move_folder(root, "Туры", "Туры/Поиск").is_err());
        assert_eq!(Collection::open(root).unwrap().folders, ["env 2", "Туры", "Туры/Заказы", "Туры/Поиск", "Туры 2"]);
        delete_folder(root, "Туры").unwrap();
        assert!(ids(root).is_empty());
        assert!(delete_folder(root, "").is_err());
        assert!(delete_folder(root, "../x").is_err());
    }
}
