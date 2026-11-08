use std::path::{Component, Path, PathBuf};

use crate::Manifest;

pub(crate) fn resolve_path(root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

pub(crate) fn state_root_for_manifest(code_root: &Path, manifest: &Manifest) -> PathBuf {
    manifest
        .plugroot
        .as_ref()
        .and_then(|config| config.state_root.as_deref().or(config.root.as_deref()))
        .map(|path| resolve_path(code_root, path))
        .unwrap_or_else(|| default_state_root(code_root))
}

pub(crate) fn default_state_root(code_root: &Path) -> PathBuf {
    let code_root = clean_path(code_root);
    let name = code_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("plugroot");
    code_root
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(format!("{name}-state"))
}

pub(crate) fn clean_path(path: &Path) -> PathBuf {
    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                clean.pop();
            }
            Component::Normal(part) => clean.push(part),
            Component::RootDir | Component::Prefix(_) => clean.push(component.as_os_str()),
        }
    }
    clean
}

pub(crate) fn same_or_inside(parent: &Path, child: &Path) -> bool {
    let parent = clean_path(parent);
    let child = clean_path(child);
    child == parent || child.starts_with(parent)
}

pub(crate) fn nearest_git_root(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().unwrap_or(path).to_path_buf()
    };

    loop {
        if current.join(".git").exists() {
            return Some(clean_path(&current));
        }
        if !current.pop() {
            return None;
        }
    }
}
