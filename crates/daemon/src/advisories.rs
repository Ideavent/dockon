use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const ECOSYSTEMS: &[&str] = &["npm", "PyPI", "crates.io", "Go", "Packagist", "NuGet"];

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    vulns: Vec<Advisory>,
    next_page_token: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct Advisory {
    id: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    aliases: Vec<String>,
    withdrawn: Option<String>,
}

pub async fn lookup(ecosystem: &str, package: &str, version: &str) -> Result<Value> {
    if !ECOSYSTEMS.contains(&ecosystem)
        || package.is_empty()
        || package.len() > 200
        || !package
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@/._-".contains(&b))
        || version.is_empty()
        || version.len() > 96
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".+_-!".contains(&b))
    {
        bail!("Enter a supported ecosystem, public package name and exact version");
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?;
    let mut response = client
        .post("https://api.osv.dev/v1/query")
        .json(&json!({"package": {"ecosystem": ecosystem, "name": package}, "version": version}))
        .send()
        .await?
        .error_for_status()?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > 2 * 1024 * 1024 {
            bail!("OSV response exceeds 2 MiB");
        }
        body.extend_from_slice(&chunk);
    }
    let page: Page = serde_json::from_slice(&body)?;
    let mut advisories: Vec<_> = page
        .vulns
        .into_iter()
        .filter(|v| v.withdrawn.is_none())
        .collect();
    let partial = page.next_page_token.is_some() || advisories.len() > 100;
    advisories.truncate(100);
    for entry in &mut advisories {
        entry.summary = entry.summary.chars().take(350).collect();
        entry.aliases.truncate(16);
    }
    Ok(json!({"source": "OSV", "partial": partial, "advisories": advisories}))
}
