//! Where projects live. Every project has one home, `~/.glasstarn/projects`,
//! unless the person asked for another folder. Those other folders are
//! remembered in `~/.glasstarn/folders.json`, so the projects page lists every
//! project wherever it was saved.

use std::path::{Path, PathBuf};

/// The default home for projects: `~/.glasstarn/projects`.
pub fn default_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".glasstarn").join("projects"))
}

fn list_path() -> Option<PathBuf> {
    Some(default_root()?.parent()?.join("folders.json"))
}

/// Other project folders that have been used, which still exist.
pub fn remembered_roots() -> Vec<PathBuf> {
    let Some(path) = list_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<PathBuf>>(&text)
        .unwrap_or_default()
        .into_iter()
        .filter(|root| root.is_dir())
        .collect()
}

/// Remembers a project folder other than the default home. Best effort: a
/// folder that cannot be written (an agent's sandbox) is simply not listed.
pub fn remember_root(root: &Path) {
    let Some(path) = list_path() else {
        return;
    };
    let Ok(root) = root.canonicalize() else {
        return;
    };
    // Scratch folders come and go; they are not where people keep projects
    // (unless the home itself is a scratch folder, as in a trial run).
    let temp = std::env::temp_dir().canonicalize().ok();
    let in_temp = |path: &Path| temp.as_ref().is_some_and(|temp| path.starts_with(temp));
    let home_in_temp = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .and_then(|home| PathBuf::from(home).canonicalize().ok())
        .is_some_and(|home| in_temp(&home));
    if in_temp(&root) && !home_in_temp {
        return;
    }
    if default_root()
        .and_then(|home| home.canonicalize().ok())
        .is_some_and(|home| home == root)
    {
        return;
    }
    let mut roots = remembered_roots();
    if roots
        .iter()
        .any(|known| known.canonicalize().is_ok_and(|known| known == root))
    {
        return;
    }
    roots.push(root);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(&roots) {
        let _ = std::fs::write(path, text + "\n");
    }
}

/// Every folder that holds projects: the given main one first, then the
/// remembered ones.
pub fn all_roots(main: &Path) -> Vec<PathBuf> {
    let mut roots = vec![main.to_path_buf()];
    for root in remembered_roots() {
        let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => a == b,
        };
        if !roots.iter().any(|known| same(known, &root)) {
            roots.push(root);
        }
    }
    roots
}

/// Moves a folder to the computer's Trash (Recycle Bin is not reachable
/// without a system call, so on Windows it goes to `~/.glasstarn/trash`),
/// where the person can still get it back. Returns where it went.
pub fn move_to_trash(path: &Path, label: &str) -> std::io::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| std::io::Error::other("cannot find your home folder"))?;
    let trash = if cfg!(target_os = "macos") {
        home.join(".Trash")
    } else if cfg!(windows) {
        home.join(".glasstarn").join("trash")
    } else {
        home.join(".local")
            .join("share")
            .join("Trash")
            .join("files")
    };
    std::fs::create_dir_all(&trash)?;
    let clean: String = label
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut target = trash.join(format!("{} (glasstarn project)", clean.trim()));
    let mut n = 2;
    while target.exists() {
        target = trash.join(format!("{} (glasstarn project {n})", clean.trim()));
        n += 1;
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}
