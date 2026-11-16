use anyhow::{bail, Context, Result};
use reqwest::Url;
use serde::de::DeserializeOwned;
use std::time::Duration;

pub fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "nodejs.org"
                    | "www.php.net"
                    | "windows.php.net"
                    | "downloads.php.net"
                    | "builds.dotnet.microsoft.com"
                    | "dotnetcli.azureedge.net"
                    | "download.visualstudio.microsoft.com"
                    | "go.dev"
                    | "dl.google.com"
                    | "api.github.com"
                    | "github.com"
                    | "codeload.github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "raw.githubusercontent.com"
                    | "www.apachelounge.com"
                    | "nginx.org"
                    | "getcomposer.org"
                    | "api.adoptium.net"
                    | "storage.googleapis.com"
                    | "repo.packagist.org"
                    | "pypi.org"
                    | "start.spring.io"
                    | "developer.android.com"
            )
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restricts_artifact_origins_and_credentials() {
        assert!(allowed(
            &Url::parse("https://nodejs.org/dist/index.json").unwrap()
        ));
        for value in [
            "http://nodejs.org/x",
            "https://localhost/x",
            "https://nodejs.org.evil.test/x",
            "https://token@nodejs.org/x",
            "https://nodejs.org:8443/x",
        ] {
            assert!(!allowed(&Url::parse(value).unwrap()));
        }
    }
}
#[derive(Clone)]
pub struct Transport(pub reqwest::Client);
impl Transport {
    pub fn new() -> Result<Self> {
        Ok(Self(
            reqwest::Client::builder()
                .user_agent("dockon/0.1.0")
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(180))
                .redirect(reqwest::redirect::Policy::custom(|attempt| {
                    if attempt.previous().len() >= 5 || !allowed(attempt.url()) {
                        attempt.stop()
                    } else {
                        attempt.follow()
                    }
                }))
                .build()?,
        ))
    }
    pub async fn response(&self, url: &str) -> Result<reqwest::Response> {
        let parsed = Url::parse(url)?;
        if !allowed(&parsed) {
            bail!("Artifact origin is not allowlisted");
        }
        let response = self
            .0
            .get(parsed.clone())
            .timeout(
                if parsed.host_str() == Some("storage.googleapis.com")
                    && parsed.path().ends_with(".zip")
                {
                    Duration::from_secs(3600)
                } else {
                    Duration::from_secs(180)
                },
            )
            .send()
            .await
            .context("Publisher request failed")?
            .error_for_status()?;
        if response.status().is_redirection() {
            bail!("Publisher redirected outside the approved origin list");
        }
        Ok(response)
    }
    pub async fn bytes(&self, url: &str, limit: usize) -> Result<Vec<u8>> {
        let mut response = self.response(url).await?;
        let mut bytes = vec![];
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > limit {
                bail!("Publisher metadata exceeds size limit");
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    pub async fn json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        Ok(serde_json::from_slice(
            &self.bytes(url, 16 * 1024 * 1024).await?,
        )?)
    }
    pub async fn text(&self, url: &str) -> Result<String> {
        Ok(String::from_utf8(self.bytes(url, 1024 * 1024).await?)?)
    }
}
