use anyhow::{bail, Context, Result};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
pub(crate) const MAX_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_FILES: usize = 100_000;

pub fn relative(path: &Path, parents: bool) -> Result<PathBuf> {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => {
                let text = part.to_str().context("Non-UTF8 archive path")?;
                if text.contains(['\\', ':']) || text.chars().any(char::is_control) {
                    bail!("Invalid archive path component");
                }
                #[cfg(windows)]
                {
                    let stem = text
                        .split('.')
                        .next()
                        .unwrap_or_default()
                        .to_ascii_uppercase();
                    if text.ends_with(['.', ' '])
                        || matches!(
                            stem.as_str(),
                            "CON"
                                | "PRN"
                                | "AUX"
                                | "NUL"
                                | "COM1"
                                | "COM2"
                                | "COM3"
                                | "COM4"
                                | "COM5"
                                | "COM6"
                                | "COM7"
                                | "COM8"
                                | "COM9"
                                | "LPT1"
                                | "LPT2"
                                | "LPT3"
                                | "LPT4"
                                | "LPT5"
                                | "LPT6"
                                | "LPT7"
                                | "LPT8"
                                | "LPT9"
                        )
                    {
                        bail!("Reserved Windows archive filename");
                    }
                }
                output.push(part);
            }
            Component::ParentDir if parents => {
                if !output.pop() {
                    bail!("Archive link escapes extraction root");
                }
            }
            _ => bail!("Archive path escapes extraction root"),
        }
    }
    Ok(output)
}
fn stripped(path: &Path, prefix: &str) -> Result<PathBuf> {
    let path = relative(path, false)?;
    if prefix.is_empty() {
        return Ok(path);
    }
    Ok(path
        .strip_prefix(prefix)
        .context("Unexpected runtime archive root")?
        .into())
}
struct Link {
    destination: PathBuf,
    target: PathBuf,
    hard: bool,
}
fn output_file(root: &Path, path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(root.join(parent))?;
    }
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join(path))?)
}
fn permissions(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o777))?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}
fn copy_limited(mut reader: impl Read, output: &mut File, size: u64) -> Result<()> {
    let bytes = std::io::copy(&mut reader.by_ref().take(size + 1), output)?;
    if bytes != size {
        bail!("Archive entry size mismatch");
    }
    output.flush()?;
    Ok(())
}

pub fn extract(archive: &Path, format: &str, prefix: &str, destination: &Path) -> Result<()> {
    extract_with_limit(archive, format, prefix, destination, MAX_BYTES)
}

pub fn extract_with_limit(
    archive: &Path,
    format: &str,
    prefix: &str,
    destination: &Path,
    byte_limit: u64,
) -> Result<()> {
    std::fs::create_dir(destination)?;
    let root = destination.canonicalize()?;
    let mut bytes = 0u64;
    let mut links = vec![];
    if format == "zip" {
        let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
        if zip.len() > MAX_FILES {
            bail!("Archive has too many entries");
        }
        for index in 0..zip.len() {
            let mut entry = zip.by_index(index)?;
            let enclosed = entry.enclosed_name().context("Unsafe ZIP path")?;
            let path = stripped(&enclosed, prefix)?;
            if path.as_os_str().is_empty() {
                continue;
            }
            bytes = bytes
                .checked_add(entry.size())
                .context("Archive size overflow")?;
            if bytes > byte_limit {
                bail!("Uncompressed runtime exceeds its extraction limit");
            }
            if entry.encrypted() {
                bail!("Encrypted runtime archives are not supported");
            }
            if entry.is_dir() {
                std::fs::create_dir_all(root.join(path))?;
                continue;
            }
            if entry.is_symlink() {
                let mut target = String::new();
                entry.by_ref().take(4097).read_to_string(&mut target)?;
                if target.len() > 4096 {
                    bail!("Archive link is too long");
                }
                let target = relative(&path.parent().unwrap_or(Path::new("")).join(target), true)?;
                links.push(Link {
                    destination: path,
                    target,
                    hard: false,
                });
                continue;
            }
            let size = entry.size();
            let mode = entry.unix_mode().unwrap_or(0o644);
            copy_limited(&mut entry, &mut output_file(&root, &path)?, size)?;
            permissions(&root.join(path), mode)?;
        }
    } else if format == "tar.gz" {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(archive)?));
        for (count, entry) in tar.entries()?.enumerate() {
            if count >= MAX_FILES {
                bail!("Archive has too many entries");
            }
            let mut entry = entry?;
            let original = entry.path()?.into_owned();
            let path = stripped(&original, prefix)?;
            if path.as_os_str().is_empty() {
                continue;
            }
            let kind = entry.header().entry_type();
            let size = entry.header().size()?;
            bytes = bytes.checked_add(size).context("Archive size overflow")?;
            if bytes > byte_limit {
                bail!("Uncompressed runtime exceeds its extraction limit");
            }
            if kind.is_dir() {
                std::fs::create_dir_all(root.join(path))?;
            } else if kind.is_file() {
                let mode = entry.header().mode()?;
                copy_limited(&mut entry, &mut output_file(&root, &path)?, size)?;
                permissions(&root.join(path), mode)?;
            } else if kind.is_symlink() || kind.is_hard_link() {
                let target = entry
                    .link_name()?
                    .context("Archive link has no target")?
                    .into_owned();
                let target = if kind.is_hard_link() {
                    stripped(&target, prefix)?
                } else {
                    relative(&path.parent().unwrap_or(Path::new("")).join(target), true)?
                };
                links.push(Link {
                    destination: path,
                    target,
                    hard: kind.is_hard_link(),
                });
            } else {
                bail!("Unsupported archive entry type");
            }
        }
    } else {
        bail!("Unsupported runtime archive format");
    }
    // Links are created only after all regular entries, so archive entries
    // cannot write through a link into an unrelated directory.
    while !links.is_empty() {
        let before = links.len();
        let mut pending = vec![];
        for link in links {
            let target = root.join(&link.target);
            if !target.exists() {
                pending.push(link);
                continue;
            }
            if !target.canonicalize()?.starts_with(&root) {
                bail!("Archive link escapes extraction root");
            }
            let destination = root.join(&link.destination);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
                if !parent.canonicalize()?.starts_with(&root) {
                    bail!("Archive link parent escapes root");
                }
            }
            if link.hard {
                std::fs::hard_link(target, destination)?;
            } else {
                #[cfg(unix)]
                std::os::unix::fs::symlink(
                    link_target(&link.destination, &link.target),
                    destination,
                )?;
                #[cfg(not(unix))]
                {
                    bail!("Symbolic links in managed Windows archives are unsupported");
                }
            }
        }
        links = pending;
        if links.len() == before {
            bail!("Unresolved/cyclic archive links");
        }
    }
    Ok(())
}

#[cfg(unix)]
fn link_target(destination: &Path, target: &Path) -> PathBuf {
    let from: Vec<_> = destination
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .collect();
    let to: Vec<_> = target.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut result = PathBuf::new();
    for _ in common..from.len() {
        result.push("..");
    }
    for component in &to[common..] {
        result.push(component.as_os_str());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_paths_and_links_escaping_the_root() {
        assert!(relative(Path::new("../outside"), false).is_err());
        assert!(relative(Path::new("/absolute"), false).is_err());
        assert!(relative(Path::new("C:\\outside"), false).is_err());
        assert!(relative(Path::new("bin/../../outside"), true).is_err());
        assert_eq!(
            relative(Path::new("bin/../lib/npm.js"), true).unwrap(),
            PathBuf::from("lib/npm.js")
        );
    }
    #[cfg(unix)]
    #[test]
    fn relative_links_survive_atomic_directory_move() {
        let directory =
            std::env::temp_dir().join(format!("dockon-archive-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let package = directory.join("sample.tar.gz");
        let file = File::create(&package).unwrap();
        let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        let data = b"verified";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "pkg/lib/target.txt", &data[..])
            .unwrap();
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        link.set_mode(0o777);
        link.set_link_name("../lib/target.txt").unwrap();
        link.set_cksum();
        tar.append_data(&mut link, "pkg/bin/link", std::io::empty())
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let staged = directory.join("staged");
        extract(&package, "tar.gz", "pkg", &staged).unwrap();
        let installed = directory.join("installed");
        std::fs::rename(staged, &installed).unwrap();
        assert_eq!(std::fs::read(installed.join("bin/link")).unwrap(), data);
        assert!(directory
            .canonicalize()
            .unwrap()
            .starts_with(std::env::temp_dir().canonicalize().unwrap()));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
