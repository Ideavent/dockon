use crate::{
    catalog::{Artifact, Version},
    transport::Transport,
};
use anyhow::{bail, Context, Result};
use serde_json::Value;

const FLUTTER_RELEASES: &str = "https://storage.googleapis.com/flutter_infra_release/releases/";
const ADOPTIUM: &str = "https://api.adoptium.net/v3";

pub async fn artifacts(http: &Transport, runtime: &str) -> Result<Vec<Artifact>> {
    let (os, arch) = crate::catalog::node_platform()?;
    let platform = crate::catalog::platform();
    let mut result = Vec::new();
    match runtime {
        "android-tools" if os == "win" => {
            let page = http
                .bytes(
                    "https://developer.android.com/studio?hl=en",
                    8 * 1024 * 1024,
                )
                .await?;
            let html = String::from_utf8(page)?;
            let (archive, build, hash) = html.split("</tr>").find_map(|row| {
                let (_, trailing) = row.split_once("commandlinetools-win-")?;
                let (build, _) = trailing.split_once("_latest.zip")?;
                if build.len() > 12 || !build.bytes().all(|b| b.is_ascii_digit()) || build.is_empty() { return None; }
                let hash = row.split(|ch: char| !ch.is_ascii_hexdigit()).find(|token| token.len() == 64)?;
                Some((format!("commandlinetools-win-{build}_latest.zip"), build, hash))
            }).context("Android Studio did not publish a Windows command-line tools download with SHA-256")?;
            result.push(Artifact {
                runtime: runtime.into(),
                version: format!("{build}.0.0"),
                platform: platform.clone(),
                url: format!("https://dl.google.com/android/repository/{archive}"),
                hash: hash.into(),
                hash_kind: "sha256".into(),
                format: "zip".into(),
                prefix: "cmdline-tools".into(),
                executable: "bin/sdkmanager.bat".into(),
                support: "publisher_release".into(),
            });
        }
        "android-tools" => {}
        "python" => {
            let triple = match (os, arch) {
                ("win", "x64") => "x86_64-pc-windows-msvc",
                ("win", "arm64") => "aarch64-pc-windows-msvc",
                ("darwin", "x64") => "x86_64-apple-darwin",
                ("darwin", "arm64") => "aarch64-apple-darwin",
                ("linux", "x64") => "x86_64-unknown-linux-gnu",
                ("linux", "arm64") => "aarch64-unknown-linux-gnu",
                _ => return Ok(result),
            };
            let release: Value = http.json("https://api.github.com/repos/astral-sh/python-build-standalone/releases/latest").await?;
            for asset in release["assets"]
                .as_array()
                .context("Python release assets missing")?
            {
                let Some(name) = asset["name"].as_str() else {
                    continue;
                };
                let Some(version) = name
                    .strip_prefix("cpython-")
                    .and_then(|name| name.split_once('+'))
                    .filter(|(_, build)| {
                        build.ends_with(&format!("-{triple}-install_only_stripped.tar.gz"))
                    })
                    .and_then(|(version, _)| semver::Version::parse(version).ok())
                else {
                    continue;
                };
                if !version.pre.is_empty() {
                    continue;
                }
                let (Some(link), Some(hash)) = (
                    asset["browser_download_url"].as_str(),
                    asset["digest"]
                        .as_str()
                        .and_then(|digest| digest.strip_prefix("sha256:")),
                ) else {
                    continue;
                };
                result.push(Artifact {
                    runtime: runtime.into(),
                    version: version.to_string(),
                    platform: platform.clone(),
                    url: link.into(),
                    hash: hash.into(),
                    hash_kind: "sha256".into(),
                    format: "tar.gz".into(),
                    prefix: "python".into(),
                    executable: if os == "win" {
                        "python.exe"
                    } else {
                        "bin/python3"
                    }
                    .into(),
                    support: "stable".into(),
                });
            }
        }
        "composer" => {
            let releases: Vec<Value> = http
                .json("https://api.github.com/repos/composer/composer/releases?per_page=40")
                .await?;
            for release in releases {
                if release["prerelease"].as_bool() != Some(false)
                    || release["draft"].as_bool() != Some(false)
                {
                    continue;
                }
                let Some(version) = release["tag_name"]
                    .as_str()
                    .and_then(|tag| semver::Version::parse(tag.trim_start_matches('v')).ok())
                else {
                    continue;
                };
                if version.major < 2 || !version.pre.is_empty() {
                    continue;
                }
                result.push(Artifact {
                    runtime: runtime.into(),
                    version: version.to_string(),
                    platform: platform.clone(),
                    url: format!("https://getcomposer.org/download/{version}/composer.phar"),
                    hash: String::new(),
                    hash_kind: "sha256".into(),
                    format: "file".into(),
                    prefix: String::new(),
                    executable: "composer.phar".into(),
                    support: "stable".into(),
                });
            }
        }
        "flutter" if (os, arch) == ("win", "x64") => {
            let index: Value = http
                .json(&format!("{FLUTTER_RELEASES}releases_windows.json"))
                .await?;
            for release in index["releases"]
                .as_array()
                .context("Invalid Flutter release catalog")?
            {
                let Some(version) = release["version"].as_str() else {
                    continue;
                };
                if release["channel"].as_str() != Some("stable")
                    || release["dart_sdk_arch"]
                        .as_str()
                        .is_some_and(|arch| arch != "x64")
                    || !semver::Version::parse(version).is_ok_and(|v| v.pre.is_empty())
                {
                    continue;
                }
                let (Some(path), Some(hash)) =
                    (release["archive"].as_str(), release["sha256"].as_str())
                else {
                    continue;
                };
                if !path.starts_with("stable/windows/flutter_windows_") || !path.ends_with(".zip") {
                    continue;
                }
                result.push(Artifact {
                    runtime: runtime.into(),
                    version: version.into(),
                    platform: platform.clone(),
                    url: format!("{FLUTTER_RELEASES}{path}"),
                    hash: hash.into(),
                    hash_kind: "sha256".into(),
                    format: "zip".into(),
                    prefix: "flutter".into(),
                    executable: "bin/flutter.bat".into(),
                    support: "stable".into(),
                });
            }
        }
        "java" => {
            let os = match os {
                "win" => "windows",
                "darwin" => "mac",
                other => other,
            };
            let arch = if arch == "arm64" { "aarch64" } else { arch };
            let supported: Value = http
                .json(&format!("{ADOPTIUM}/info/available_releases"))
                .await?;
            let latest = supported["most_recent_feature_release"]
                .as_u64()
                .unwrap_or(25);
            let mut features = vec![17, 21, 25, latest];
            features.sort_unstable();
            features.dedup();
            for feature in features {
                let url = format!("{ADOPTIUM}/assets/feature_releases/{feature}/ga?architecture={arch}&heap_size=normal&image_type=jdk&jvm_impl=hotspot&os={os}&page_size=5&vendor=eclipse");
                let releases: Vec<Value> = http.json(&url).await?;
                for release in releases {
                    let (Some(label), Some(release_name)) = (
                        release["version_data"]["semver"].as_str(),
                        release["release_name"].as_str(),
                    ) else {
                        continue;
                    };
                    let Ok(version) = semver::Version::parse(label) else {
                        continue;
                    };
                    let Some(binary) = release["binaries"].as_array().and_then(|items| {
                        items.iter().find(|v| {
                            v["package"]["link"].as_str().is_some_and(|link| {
                                link.ends_with(if os == "windows" { ".zip" } else { ".tar.gz" })
                            })
                        })
                    }) else {
                        continue;
                    };
                    let package = &binary["package"];
                    let (Some(link), Some(hash)) =
                        (package["link"].as_str(), package["checksum"].as_str())
                    else {
                        continue;
                    };
                    result.push(Artifact {
                        runtime: runtime.into(),
                        version: version.to_string(),
                        platform: platform.clone(),
                        url: link.into(),
                        hash: hash.into(),
                        hash_kind: "sha256".into(),
                        format: if os == "windows" { "zip" } else { "tar.gz" }.into(),
                        prefix: release_name.into(),
                        executable: match os {
                            "windows" => "bin/java.exe",
                            "mac" => "Contents/Home/bin/java",
                            _ => "bin/java",
                        }
                        .into(),
                        support: if [17, 21, 25].contains(&feature) {
                            "lts"
                        } else {
                            "current"
                        }
                        .into(),
                    });
                }
            }
        }
        "flutter" => {}
        _ => bail!("Unsupported SDK catalog"),
    }
    result.sort_by_key(|value| std::cmp::Reverse(semver::Version::parse(&value.version).ok()));
    result.dedup_by(|a, b| a.version == b.version);
    Ok(result)
}

pub async fn versions(http: &Transport, runtime: &str) -> Result<Vec<Version>> {
    Ok(artifacts(http, runtime)
        .await?
        .into_iter()
        .map(|row| Version {
            version: row.version,
            support: row.support,
        })
        .collect())
}

pub async fn artifact(http: &Transport, runtime: &str, version: &str) -> Result<Artifact> {
    let mut asset = artifacts(http, runtime)
        .await?
        .into_iter()
        .find(|item| item.version == version)
        .context("Requested SDK version is unavailable for this platform")?;
    if runtime == "composer" {
        let digest = http.text(&format!("{}.sha256sum", asset.url)).await?;
        asset.hash = digest
            .split_whitespace()
            .next()
            .context("Composer checksum is missing")?
            .into();
    }
    Ok(asset)
}
