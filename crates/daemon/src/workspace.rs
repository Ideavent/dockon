use anyhow::{bail, Result};
use serde::Serialize;
use std::{fs, io::ErrorKind, path::PathBuf};

#[derive(Serialize)]
pub struct Workspace {
    root: PathBuf,
    exists: bool,
    folders: Vec<ProjectFolder>,
}

#[derive(Serialize)]
struct ProjectFolder {
    name: String,
    path: PathBuf,
}

pub fn folders() -> Result<Workspace> {
    scan(root()?)
}

pub fn root() -> Result<PathBuf> {
    let root = match std::env::var_os("DOCKON_WWW_DIR") {
        Some(path) => PathBuf::from(path),
        None if cfg!(windows) => PathBuf::from(r"C:\dockon\www"),
        None => dockon_ipc::data_dir()?.join("www"),
    };
    if !root.is_absolute() {
        bail!("The www directory must be an absolute path");
    }
    Ok(root)
}

pub fn project_path(slug: &str) -> Result<PathBuf> {
    if slug.is_empty()
        || slug.len() > 63
        || slug.starts_with('-')
        || slug.ends_with('-')
        || !slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        bail!("Use a folder name with lowercase letters, digits and hyphens");
    }
    let root = root()?;
    fs::create_dir_all(&root)?;
    Ok(root.join(slug))
}

fn scan(root: PathBuf) -> Result<Workspace> {
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(Workspace {
                root,
                exists: false,
                folders: Vec::new(),
            });
        }
        Err(error) => return Err(error.into()),
    };
    let canonical = root.canonicalize()?;
    let mut folders = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() || is_reparse_point(&entry)? {
            continue;
        }
        let resolved = match entry.path().canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if resolved.parent() != Some(canonical.as_path())
            || resolved.file_name() != Some(entry.file_name().as_os_str())
        {
            continue;
        }
        folders.push(ProjectFolder {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: entry.path(),
        });
    }
    folders.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Workspace {
        root,
        exists: true,
        folders,
    })
}

#[cfg(windows)]
fn is_reparse_point(entry: &fs::DirEntry) -> Result<bool> {
    use std::os::windows::fs::MetadataExt;
    Ok(entry.metadata()?.file_attributes() & 0x400 != 0)
}

#[cfg(not(windows))]
fn is_reparse_point(_: &fs::DirEntry) -> Result<bool> {
    Ok(false)
}
