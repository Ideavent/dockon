mod archive;
pub mod catalog;
mod profiles;
pub mod projects;
mod transport;
use anyhow::{bail, Context, Result};
use dockon_core::RuntimeInstallation;
use dockon_persistence::Store;
use serde::Serialize;
use sha2::{Digest, Sha256, Sha512};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

pub type Progress = Arc<dyn Fn(String) + Send + Sync>;
pub struct RuntimeManager {
    root: PathBuf,
    http: transport::Transport,
    store: Store,
    install_lock: Mutex<()>,
}
#[derive(Debug, Serialize)]
pub struct SystemCandidate {
    pub runtime: String,
    pub executable: PathBuf,
    pub version_checked: bool,
}

#[derive(Serialize)]
pub struct RuntimeUpdate {
    installation_id: String,
    current_version: String,
    version: String,
    same_major: bool,
    already_installed: bool,
}

impl RuntimeManager {
    pub fn new(root: PathBuf, store: Store) -> Result<Self> {
        Ok(Self {
            root,
            http: transport::Transport::new()?,
            store,
            install_lock: Mutex::new(()),
        })
    }
    pub async fn installed(&self) -> Result<Vec<RuntimeInstallation>> {
        Ok(self
            .store
            .runtimes()
            .await?
            .into_iter()
            .map(|mut record| {
                record.executable = dunce::simplified(&record.executable).to_path_buf();
                record.root = dunce::simplified(&record.root).to_path_buf();
                record
            })
            .collect())
    }
    pub async fn catalog(&self, runtime: &str) -> Result<catalog::Catalog> {
        runtime_id(runtime)?;
        let cache = self
            .root
            .join("catalogs")
            .join(format!("{runtime}-{}.json", catalog::platform()));
        let fetched = tokio::time::timeout(
            std::time::Duration::from_secs(25),
            catalog::versions(&self.http, runtime),
        )
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("Publisher catalog request timed out")));
        match fetched {
            Ok((versions, note)) => {
                let value = catalog::Catalog {
                    runtime: runtime.into(),
                    platform: catalog::platform(),
                    versions,
                    note,
                    cached: false,
                    fetched_at: chrono::Utc::now().to_rfc3339(),
                };
                std::fs::create_dir_all(cache.parent().context("Missing cache directory")?)?;
                std::fs::write(&cache, serde_json::to_vec(&value)?)?;
                Ok(value)
            }
            Err(error) => {
                if let Ok(bytes) = std::fs::read(cache) {
                    if let Ok(mut catalog) = serde_json::from_slice::<catalog::Catalog>(&bytes) {
                        catalog.cached = true;
                        catalog.note =
                            Some(format!("Publisher unavailable; cached catalog: {error}"));
                        return Ok(catalog);
                    }
                }
                Err(error)
            }
        }
    }

    pub async fn updates(&self, runtime: &str) -> Result<serde_json::Value> {
        let catalog = self.catalog(runtime).await?;
        let installations: Vec<_> = self
            .installed()
            .await?
            .into_iter()
            .filter(|entry| entry.runtime == runtime)
            .collect();
        let candidates: Vec<_> = catalog
            .versions
            .iter()
            .map(|entry| entry.version.as_str())
            .collect();
        let mut updates = Vec::new();
        for entry in &installations {
            if let Some((candidate, same_major)) =
                catalog::suggest_update(&entry.version, &candidates)
            {
                updates.push(RuntimeUpdate {
                    installation_id: entry.id.clone(),
                    current_version: entry.version.clone(),
                    version: candidate.to_owned(),
                    same_major,
                    already_installed: installations
                        .iter()
                        .any(|installed| installed.version == candidate),
                });
            }
        }
        Ok(serde_json::json!({"cached": catalog.cached, "updates": updates}))
    }
    pub fn discover(&self) -> Vec<SystemCandidate> {
        let mut found = vec![];
        let mut seen = std::collections::BTreeSet::new();
        for runtime in [
            "node", "php", "php-cgi", "dotnet", "go", "python", "rust", "caddy", "apache", "nginx",
        ] {
            let name = match runtime {
                "rust" => "cargo",
                "apache" => "httpd",
                other => other,
            };
            let filename = if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.into()
            };
            for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
                let candidate = directory.join(&filename);
                if !candidate.is_file() || candidate.metadata().map_or(true, |m| m.len() == 0) {
                    continue;
                }
                if let Ok(executable) = candidate.canonicalize() {
                    if seen.insert(executable.clone()) {
                        found.push(SystemCandidate {
                            runtime: runtime.into(),
                            executable,
                            version_checked: false,
                        });
                    }
                }
            }
        }
        found
    }
    pub async fn use_system(
        &self,
        runtime: &str,
        executable: &Path,
    ) -> Result<RuntimeInstallation> {
        self.register_executable(runtime, executable, false).await
    }

    pub async fn register_bundled_web(&self, directory: &Path) -> Result<()> {
        let _guard = self.install_lock.lock().await;
        let installed = self.installed().await?;
        for (runtime, relative) in [
            ("apache", "bin/apache/bin/httpd.exe"),
            ("php", "bin/php/php.exe"),
            ("php-cgi", "bin/php/php-cgi.exe"),
        ] {
            let executable = directory.join(relative);
            if executable.is_file()
                && !installed.iter().any(|record| {
                    record.runtime == runtime
                        && record.source == "dockon_bundle"
                        && record.executable == executable
                })
            {
                self.register_executable(runtime, &executable, true).await?;
            }
        }
        Ok(())
    }

    async fn register_executable(
        &self,
        runtime: &str,
        executable: &Path,
        bundled: bool,
    ) -> Result<RuntimeInstallation> {
        runtime_id(runtime)?;
        let executable = executable.canonicalize()?;
        if !executable.is_file() {
            bail!("Select an existing runtime executable");
        }
        if cfg!(windows) && executable.extension().and_then(|v| v.to_str()) != Some("exe") {
            bail!("Select a native executable, not a shell script");
        }
        let check = self
            .root
            .join("runtime-checks")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&check)?;
        let version = inspect_version(runtime, &executable, &check).await?;
        let root = runtime_root(runtime, &executable)?;
        let hash = hash_file(&executable, "sha256")?;
        let record = RuntimeInstallation {
            id: format!(
                "{}:{runtime}:{:x}",
                if bundled { "bundle" } else { "system" },
                Sha256::digest(executable.to_string_lossy().as_bytes())
            ),
            runtime: runtime.into(),
            version,
            platform: catalog::platform(),
            root,
            executable,
            source: if bundled {
                "dockon_bundle"
            } else {
                "explicit_system_opt_in"
            }
            .into(),
            archive_hash: None,
            executable_hash: hash,
            signature_verified: false,
        };
        self.store.save_runtime(&record).await?;
        Ok(record)
    }
    pub async fn install(
        &self,
        runtime: &str,
        version: &str,
        progress: Progress,
    ) -> Result<RuntimeInstallation> {
        runtime_id(runtime)?;
        version_id(version)?;
        let _guard = self.install_lock.lock().await;
        let id = format!("managed:{runtime}:{version}:{}", catalog::platform());
        if let Some(existing) = self.installed().await?.into_iter().find(|v| v.id == id) {
            self.verify(&existing).await?;
            return Ok(existing);
        }
        let metadata = self
            .root
            .join("catalogs/artifacts")
            .join(format!("{runtime}-{version}-{}.json", catalog::platform()));
        progress(format!(
            "Resolving publisher metadata for {runtime} {version}"
        ));
        let artifact = match catalog::artifact(&self.http, runtime, version).await {
            Ok(value) => {
                std::fs::create_dir_all(metadata.parent().context("Missing metadata directory")?)?;
                std::fs::write(&metadata, serde_json::to_vec(&value)?)?;
                value
            }
            Err(error) => {
                let value: catalog::Artifact =
                    serde_json::from_slice(&std::fs::read(&metadata).with_context(|| {
                        format!("Publisher unavailable and no cached artifact metadata: {error}")
                    })?)?;
                progress("Using cached publisher metadata".into());
                value
            }
        };
        if artifact.runtime != runtime
            || artifact.version != version
            || artifact.platform != catalog::platform()
        {
            bail!("Cached artifact identity mismatch");
        }
        validate_digest(&artifact.hash, &artifact.hash_kind)?;
        let downloads = self.root.join("packages/downloads");
        std::fs::create_dir_all(&downloads)?;
        let cached = downloads.join(format!("{}.archive", artifact.hash.to_ascii_lowercase()));
        if !cached.is_file() {
            let temporary = downloads.join(format!("{}.part", uuid::Uuid::new_v4()));
            let mut response = self.http.response(&artifact.url).await?;
            if response
                .content_length()
                .is_some_and(|size| size > 2 * 1024 * 1024 * 1024)
            {
                bail!("Runtime archive exceeds 2 GiB download limit");
            }
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .await?;
            let mut count = 0u64;
            let mut reported = 0u64;
            while let Some(chunk) = response.chunk().await? {
                count += chunk.len() as u64;
                if count > 2 * 1024 * 1024 * 1024 {
                    bail!("Runtime archive exceeds 2 GiB download limit");
                }
                file.write_all(&chunk).await?;
                if count - reported >= 4 * 1024 * 1024 {
                    progress(format!("Downloaded {count} bytes"));
                    reported = count;
                }
            }
            file.flush().await?;
            file.sync_all().await?;
            drop(file);
            progress(format!(
                "Verifying {} digest ({count} bytes)",
                artifact.hash_kind
            ));
            if !hash_file(&temporary, &artifact.hash_kind)?.eq_ignore_ascii_case(&artifact.hash) {
                bail!("Publisher artifact digest mismatch; archive retained in quarantine");
            }
            tokio::fs::rename(temporary, &cached).await?;
        }
        if !hash_file(&cached, &artifact.hash_kind)?.eq_ignore_ascii_case(&artifact.hash) {
            bail!("Cached runtime archive failed integrity verification");
        }
        let stages = self.root.join("packages/staging");
        std::fs::create_dir_all(&stages)?;
        let stage = stages.join(uuid::Uuid::new_v4().to_string());
        progress("Extracting verified runtime in a private staging directory".into());
        let archive_path = cached.clone();
        let format = artifact.format.clone();
        let prefix = artifact.prefix.clone();
        let destination = stage.clone();
        tokio::task::spawn_blocking(move || {
            archive::extract(&archive_path, &format, &prefix, &destination)
        })
        .await??;
        let executable = stage.join(archive::relative(Path::new(&artifact.executable), false)?);
        if !executable.is_file()
            || !executable
                .canonicalize()?
                .starts_with(stage.canonicalize()?)
        {
            bail!("Runtime executable missing or outside staged installation");
        }
        let check = self
            .root
            .join("runtime-checks")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&check)?;
        let actual = inspect_version(runtime, &executable, &check).await?;
        let expected = version.split('-').next().unwrap_or(version);
        if actual != expected {
            bail!("Runtime version mismatch: expected {expected}, observed {actual}");
        }
        let target = self
            .root
            .join("runtimes")
            .join(runtime)
            .join(version)
            .join(catalog::platform());
        std::fs::create_dir_all(target.parent().context("Missing runtime parent")?)?;
        if target.exists() {
            bail!("An unregistered runtime directory already exists; preserving it for recovery");
        }
        let executable_hash = hash_file(&executable, "sha256")?;
        std::fs::rename(&stage, &target)?;
        let executable = target.join(&artifact.executable).canonicalize()?;
        let record = RuntimeInstallation {
            id,
            runtime: runtime.into(),
            version: version.into(),
            platform: catalog::platform(),
            root: target.canonicalize()?,
            executable,
            source: artifact.url,
            archive_hash: Some(artifact.hash),
            executable_hash,
            signature_verified: false,
        };
        self.store.save_runtime(&record).await?;
        progress(format!("Installed and verified {runtime} {version}; publisher signature verification is not yet implemented"));
        Ok(record)
    }
    pub async fn verify(&self, record: &RuntimeInstallation) -> Result<()> {
        if record.platform != catalog::platform() {
            bail!("Runtime installation belongs to a different platform");
        }
        if !record.executable.is_file()
            || hash_file(&record.executable, "sha256")? != record.executable_hash
        {
            bail!("Runtime executable changed or is missing; reselect or repair this installation");
        }
        if record.source == "explicit_system_opt_in" {
            let directory = self.root.join("runtime-checks");
            std::fs::create_dir_all(&directory)?;
            if inspect_version(&record.runtime, &record.executable, &directory).await?
                != record.version
            {
                bail!("System runtime version changed; explicitly select it again");
            }
        }
        Ok(())
    }
    pub async fn selected(&self, id: &str) -> Result<RuntimeInstallation> {
        let record = self
            .installed()
            .await?
            .into_iter()
            .find(|v| v.id == id)
            .context("Runtime installation not found")?;
        self.verify(&record).await?;
        Ok(record)
    }
}

fn runtime_root(runtime: &str, executable: &Path) -> Result<PathBuf> {
    let parent = executable
        .parent()
        .context("Runtime directory unavailable")?;
    Ok(
        if matches!(runtime, "node" | "go") && parent.file_name().is_some_and(|v| v == "bin") {
            parent.parent().unwrap_or(parent)
        } else {
            parent
        }
        .into(),
    )
}
fn runtime_id(runtime: &str) -> Result<()> {
    if !matches!(
        runtime,
        "node"
            | "php"
            | "php-cgi"
            | "dotnet"
            | "go"
            | "python"
            | "rust"
            | "caddy"
            | "apache"
            | "nginx"
    ) {
        bail!("Unsupported runtime id");
    }
    Ok(())
}
fn version_id(version: &str) -> Result<()> {
    if version.is_empty()
        || version.len() > 64
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
    {
        bail!("Invalid runtime version");
    }
    Ok(())
}
fn validate_digest(hash: &str, kind: &str) -> Result<()> {
    let length = match kind {
        "sha256" => 64,
        "sha512" => 128,
        _ => bail!("Unsupported artifact digest"),
    };
    if hash.len() != length || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Invalid artifact digest");
    }
    Ok(())
}
pub fn hash_file(path: &Path, kind: &str) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0u8; 65536];
    let mut sha256 = Sha256::new();
    let mut sha512 = Sha512::new();
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        match kind {
            "sha256" => sha256.update(&buffer[..n]),
            "sha512" => sha512.update(&buffer[..n]),
            _ => bail!("Unsupported digest"),
        }
    }
    Ok(if kind == "sha512" {
        format!("{:x}", sha512.finalize())
    } else {
        format!("{:x}", sha256.finalize())
    })
}
pub async fn inspect_version(runtime: &str, executable: &Path, directory: &Path) -> Result<String> {
    let mut command = tokio::process::Command::new(executable);
    command
        .args(match runtime {
            "php" | "php-cgi" => vec!["-n", "--version"],
            "go" | "caddy" => vec!["version"],
            "apache" | "nginx" => vec!["-v"],
            _ => vec!["--version"],
        })
        .current_dir(directory)
        .env("DOTNET_CLI_HOME", directory)
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_GENERATE_ASPNET_CERTIFICATE", "false")
        .env_remove("NODE_OPTIONS")
        .env_remove("NODE_PATH")
        .env_remove("PYTHONHOME")
        .env_remove("PYTHONPATH")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let mut child = command.spawn()?;
        let mut bytes = vec![];
        let stdout = child.stdout.take().context("Runtime stdout unavailable")?;
        let stderr = child.stderr.take().context("Runtime stderr unavailable")?;
        let mut errors = vec![];
        tokio::try_join!(
            async { stdout.take(65537).read_to_end(&mut bytes).await },
            async { stderr.take(65537).read_to_end(&mut errors).await }
        )?;
        if bytes.len() > 65536 || errors.len() > 65536 {
            bail!("Runtime version output exceeds limit");
        }
        if !child.wait().await?.success() {
            bail!("Runtime version check failed; verify its platform prerequisites");
        }
        if runtime == "nginx" {
            bytes.push(b' ');
            bytes.extend(errors);
        }
        Ok::<_, anyhow::Error>(bytes)
    })
    .await
    .context("Runtime version check timed out")??;
    let text = String::from_utf8_lossy(&output);
    if runtime == "php-cgi" && !text.contains("cgi-fcgi") {
        bail!("Select the PHP CGI executable (php-cgi), not the PHP CLI");
    }
    if matches!(runtime, "apache" | "nginx") {
        let product = if runtime == "apache" {
            "Apache/"
        } else {
            "nginx/"
        };
        let version = text
            .split_whitespace()
            .find_map(|part| part.strip_prefix(product))
            .context("Executable did not identify itself as the selected web server")?;
        return Ok(semver::Version::parse(version)?.to_string());
    }
    let version = text
        .split_whitespace()
        .find_map(|part| {
            let part = part.trim_start_matches('v').trim_start_matches("go");
            semver::Version::parse(part).ok().map(|v| v.to_string())
        })
        .context("Runtime did not report a recognizable version")?;
    Ok(version)
}
