use crate::transport::Transport;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub runtime: String,
    pub version: String,
    pub platform: String,
    pub url: String,
    pub hash: String,
    pub hash_kind: String,
    pub format: String,
    pub prefix: String,
    pub executable: String,
    pub support: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub runtime: String,
    pub platform: String,
    pub versions: Vec<Version>,
    pub note: Option<String>,
    pub cached: bool,
    pub fetched_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    pub version: String,
    pub support: String,
}
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}
fn node_platform() -> Result<(&'static str, &'static str)> {
    let os = match std::env::consts::OS {
        "windows" => "win",
        "macos" => "darwin",
        "linux" => "linux",
        _ => bail!("Unsupported runtime platform"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => bail!("Unsupported runtime architecture"),
    };
    Ok((os, arch))
}
fn version_key(value: &str) -> semver::Version {
    semver::Version::parse(value.trim_start_matches('v').trim_start_matches("go"))
        .unwrap_or_else(|_| semver::Version::new(0, 0, 0))
}
pub async fn versions(http: &Transport, runtime: &str) -> Result<(Vec<Version>, Option<String>)> {
    let mut result=match runtime {
        "node"=>{
            let index:Vec<Value>=http.json("https://nodejs.org/dist/index.json").await?;
            let schedule:Value=http.json("https://raw.githubusercontent.com/nodejs/Release/main/schedule.json").await?;
            let today=chrono::Utc::now().format("%Y-%m-%d").to_string();
            index.into_iter().filter_map(|entry|{let value=entry["version"].as_str()?.trim_start_matches('v');let version=semver::Version::parse(value).ok()?;if !version.pre.is_empty(){return None;}
                let end=schedule[format!("v{}",version.major)]["end"].as_str().map(str::to_owned);
                Some(Version{version:value.into(),support:if end.is_some_and(|date|date<today){"eol"}else if entry["lts"].is_string(){"lts"}else{"current"}.into()})}).collect()
        }
        "php"=>{
            if std::env::consts::OS!="windows"||std::env::consts::ARCH!="x86_64"{return Ok((vec![],Some("Managed PHP binaries currently cover Windows x64 NTS; use an explicitly selected system PHP or a container on this platform".into())));}
            let index:Value=http.json("https://windows.php.net/downloads/releases/releases.json").await?;
            index.as_object().context("Invalid PHP catalog")?.values().filter_map(|entry|Some(Version{version:entry["version"].as_str()?.into(),support:"publisher_release_support_unverified".into()})).collect()
        }
        "dotnet"=>{
            let index:Value=http.json("https://builds.dotnet.microsoft.com/dotnet/release-metadata/releases-index.json").await?;
            let mut versions=BTreeMap::new();
            for channel in index["releases-index"].as_array().context("Invalid .NET catalog")?.iter().filter(|v|matches!(v["support-phase"].as_str(),Some("active"|"maintenance"))) {
                let releases:Value=http.json(channel["releases.json"].as_str().context("Missing .NET release URL")?).await?;
                for release in releases["releases"].as_array().context("Invalid .NET releases")?{
                    let sdks=release["sdks"].as_array().cloned().unwrap_or_else(||vec![release["sdk"].clone()]);
                    for sdk in sdks{if let Some(version)=sdk["version"].as_str(){if semver::Version::parse(version).is_ok_and(|v|v.pre.is_empty()){versions.insert(version.to_owned(),Version{version:version.into(),support:channel["release-type"].as_str().unwrap_or("unknown").into()});}}}
                }
            }versions.into_values().collect()
        }
        "go"=>{let index:Vec<Value>=http.json("https://go.dev/dl/?mode=json&include=all").await?;index.into_iter().filter(|v|v["stable"].as_bool()==Some(true)).filter_map(|v|Some(Version{version:v["version"].as_str()?.trim_start_matches("go").into(),support:"publisher_release".into()})).collect()}
        "caddy"=>{let release=http.response("https://github.com/caddyserver/caddy/releases/latest").await?;let version=release.url().as_str().strip_prefix("https://github.com/caddyserver/caddy/releases/tag/v").context("Unexpected Caddy release redirect")?;semver::Version::parse(version)?;vec![Version{version:version.into(),support:"publisher_release".into()}]}
        _=>return Ok((vec![],Some("Managed downloads are not implemented for this runtime; explicit system runtime import is available for supported executable profiles".into())))
    };
    result.sort_by_key(|value| std::cmp::Reverse(version_key(&value.version)));
    result.dedup_by(|a, b| a.version == b.version);
    result.truncate(200);
    Ok((result, None))
}
pub async fn artifact(http: &Transport, runtime: &str, version: &str) -> Result<Artifact> {
    let (os, arch) = node_platform()?;
    let mut value = Artifact {
        runtime: runtime.into(),
        version: version.into(),
        platform: platform(),
        url: String::new(),
        hash: String::new(),
        hash_kind: "sha256".into(),
        format: if os == "win" { "zip" } else { "tar.gz" }.into(),
        prefix: String::new(),
        executable: String::new(),
        support: "publisher_release".into(),
    };
    match runtime {
        "node" => {
            let index: Vec<Value> = http.json("https://nodejs.org/dist/index.json").await?;
            let selected = index
                .iter()
                .find(|v| v["version"].as_str() == Some(&format!("v{version}")))
                .context("Node.js version is not in the publisher catalog")?;
            if !semver::Version::parse(version)?.pre.is_empty() {
                bail!("Prerelease runtime installation is not enabled");
            }
            let file_platform = if os == "darwin" { "osx" } else { os };
            let file_kind = if os == "win" { "zip" } else { "tar" };
            let capability = if os == "linux" {
                format!("linux-{arch}")
            } else {
                format!("{file_platform}-{arch}-{file_kind}")
            };
            if !selected["files"]
                .as_array()
                .is_some_and(|v| v.iter().any(|f| f.as_str() == Some(&capability)))
            {
                bail!("Publisher has no Node.js binary for this platform");
            }
            value.prefix = format!("node-v{version}-{os}-{arch}");
            let filename = format!("{}.{}", value.prefix, value.format);
            value.url = format!("https://nodejs.org/dist/v{version}/{filename}");
            value.hash = checksum(
                &http
                    .text(&format!(
                        "https://nodejs.org/dist/v{version}/SHASUMS256.txt"
                    ))
                    .await?,
                &filename,
            )?;
            value.executable = if os == "win" { "node.exe" } else { "bin/node" }.into();
        }
        "php" => {
            if os != "win" || arch != "x64" {
                bail!("Managed PHP is unavailable on this platform");
            }
            let index: Value = http
                .json("https://windows.php.net/downloads/releases/releases.json")
                .await?;
            let selected = index
                .as_object()
                .context("Invalid PHP catalog")?
                .values()
                .find(|v| v["version"].as_str() == Some(version))
                .context("PHP version is not in the publisher catalog")?;
            let package = selected
                .as_object()
                .context("Invalid PHP release")?
                .iter()
                .find(|(key, _)| key.starts_with("nts-") && key.ends_with("-x64"))
                .map(|(_, v)| v)
                .context("PHP NTS x64 artifact unavailable")?;
            let filename = package["zip"]["path"]
                .as_str()
                .context("Missing PHP archive")?;
            safe_filename(filename)?;
            value.url = format!("https://windows.php.net/downloads/releases/{filename}");
            value.hash = package["zip"]["sha256"]
                .as_str()
                .context("Missing PHP digest")?
                .into();
            value.executable = "php.exe".into();
        }
        "dotnet" => {
            let parsed = semver::Version::parse(version)?;
            if !parsed.pre.is_empty() {
                bail!("Prerelease SDK installation is not enabled");
            }
            let releases:Value=http.json(&format!("https://builds.dotnet.microsoft.com/dotnet/release-metadata/{}.{}/releases.json",parsed.major,parsed.minor)).await?;
            let rid = format!(
                "{}-{arch}",
                match os {
                    "win" => "win",
                    "darwin" => "osx",
                    _ => "linux",
                }
            );
            let mut file = None;
            for release in releases["releases"]
                .as_array()
                .context("Invalid .NET releases")?
            {
                let sdks = release["sdks"]
                    .as_array()
                    .cloned()
                    .unwrap_or_else(|| vec![release["sdk"].clone()]);
                for sdk in sdks
                    .iter()
                    .filter(|s| s["version"].as_str() == Some(version))
                {
                    file = sdk["files"]
                        .as_array()
                        .and_then(|files| {
                            files.iter().find(|f| {
                                f["rid"].as_str() == Some(&rid)
                                    && f["url"]
                                        .as_str()
                                        .is_some_and(|u| u.ends_with(&value.format))
                            })
                        })
                        .cloned();
                    if file.is_some() {
                        break;
                    }
                }
                if file.is_some() {
                    break;
                }
            }
            let file = file.context("SDK archive unavailable for this version/platform")?;
            value.url = file["url"]
                .as_str()
                .context("Missing SDK download URL")?
                .into();
            value.hash = file["hash"]
                .as_str()
                .context("Missing SDK SHA-512 digest")?
                .into();
            value.hash_kind = "sha512".into();
            value.executable = if os == "win" { "dotnet.exe" } else { "dotnet" }.into();
        }
        "go" => {
            let index: Vec<Value> = http
                .json("https://go.dev/dl/?mode=json&include=all")
                .await?;
            let selected = index
                .iter()
                .find(|v| {
                    v["version"].as_str() == Some(&format!("go{version}"))
                        && v["stable"].as_bool() == Some(true)
                })
                .context("Go version unavailable")?;
            let go_os = if os == "win" { "windows" } else { os };
            let go_arch = if arch == "x64" { "amd64" } else { arch };
            let file = selected["files"]
                .as_array()
                .and_then(|files| {
                    files.iter().find(|f| {
                        f["os"].as_str() == Some(go_os)
                            && f["arch"].as_str() == Some(go_arch)
                            && f["kind"].as_str() == Some("archive")
                    })
                })
                .context("Go artifact unavailable on this platform")?;
            let filename = file["filename"].as_str().context("Missing Go filename")?;
            safe_filename(filename)?;
            value.url = format!("https://go.dev/dl/{filename}");
            value.hash = file["sha256"].as_str().context("Missing Go digest")?.into();
            value.prefix = "go".into();
            value.executable = if os == "win" { "bin/go.exe" } else { "bin/go" }.into();
        }
        "caddy" => {
            semver::Version::parse(version)?;
            let caddy_os = if os == "darwin" {
                "mac"
            } else if os == "win" {
                "windows"
            } else {
                os
            };
            let caddy_arch = if arch == "x64" { "amd64" } else { arch };
            let filename = format!("caddy_{version}_{caddy_os}_{caddy_arch}.{}", value.format);
            let release =
                format!("https://github.com/caddyserver/caddy/releases/download/v{version}");
            value.url = format!("{release}/{filename}");
            value.hash = checksum(
                &http
                    .text(&format!("{release}/caddy_{version}_checksums.txt"))
                    .await?,
                &filename,
            )?;
            value.executable = if os == "win" { "caddy.exe" } else { "caddy" }.into();
            // Caddy's official release manifest uses SHA-512.
            value.hash_kind = "sha512".into();
        }
        _ => bail!("Managed installer is not implemented for this runtime"),
    }
    let length = if value.hash_kind == "sha512" { 128 } else { 64 };
    if value.hash.len() != length || !value.hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Publisher supplied an invalid artifact digest");
    }
    Ok(value)
}
fn checksum(text: &str, filename: &str) -> Result<String> {
    text.lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            (name == filename).then(|| hash.to_owned())
        })
        .context("Artifact is absent from publisher checksum manifest")
}
fn safe_filename(name: &str) -> Result<()> {
    if name.is_empty()
        || name.contains(['/', '\\', ':'])
        || name.contains("..")
        || name.chars().any(char::is_control)
    {
        bail!("Invalid publisher artifact filename");
    }
    Ok(())
}
