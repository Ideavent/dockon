use super::{workspace, DaemonState};
use anyhow::{bail, Context, Result};
use dockon_core::{Operation, PortableRuntime, ProjectPreset};
use serde::Serialize;
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
};

#[derive(Serialize)]
pub struct NamedPreset {
    id: String,
    preset: ProjectPreset,
}

#[derive(Serialize)]
pub struct ProjectCheck {
    code: &'static str,
    ok: bool,
    detail: String,
    action: &'static str,
}

fn presets_dir() -> Result<PathBuf> {
    Ok(dockon_ipc::data_dir()?.join("project-presets"))
}

pub fn list_presets() -> Result<Vec<NamedPreset>> {
    let directory = presets_dir()?;
    if !directory.exists() {
        return Ok(vec![]);
    }
    let mut result = vec![];
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file()
            || path.extension().and_then(|v| v.to_str()) != Some("json")
        {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|v| v.to_str()) else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_err() {
            continue;
        }
        let mut body = Vec::new();
        std::fs::File::open(&path)?
            .take(32769)
            .read_to_end(&mut body)?;
        if body.len() > 32768 {
            continue;
        }
        if let Ok(preset) = serde_json::from_slice::<ProjectPreset>(&body) {
            if preset.validate().is_ok() {
                result.push(NamedPreset {
                    id: id.to_owned(),
                    preset,
                });
            }
        }
    }
    result.sort_by(|a, b| {
        a.preset
            .name
            .cmp(&b.preset.name)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(result)
}

pub fn save_preset(preset: ProjectPreset) -> Result<NamedPreset> {
    preset.validate()?;
    let directory = presets_dir()?;
    std::fs::create_dir_all(&directory)?;
    let id = uuid::Uuid::new_v4().to_string();
    let data = serde_json::to_vec_pretty(&preset)?;
    let partial = directory.join(format!("{id}.tmp"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    file.write_all(&data)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(partial, directory.join(format!("{id}.json")))?;
    Ok(NamedPreset { id, preset })
}

pub fn remove_preset(id: String) -> Result<()> {
    let id = uuid::Uuid::parse_str(&id)?;
    std::fs::remove_file(presets_dir()?.join(format!("{id}.json")))?;
    Ok(())
}

impl DaemonState {
    pub async fn capture_preset(&self, id: &str, name: String) -> Result<ProjectPreset> {
        let project = self
            .projects
            .list()
            .await?
            .into_iter()
            .find(|project| project.manifest.id == id)
            .context("Unknown project")?;
        let manifest = project.manifest;
        let installed = self.runtimes.installed().await?;
        let version = |id: &str| {
            installed
                .iter()
                .find(|record| record.id == id)
                .map(|record| PortableRuntime {
                    runtime: record.runtime.clone(),
                    version: record.version.clone(),
                })
        };
        let routing = self.router.status().await?.settings;
        let https = routing
            .as_ref()
            .map(|settings| {
                settings
                    .routes
                    .iter()
                    .find(|route| route.project_id == id)
                    .map_or(settings.default_https, |route| route.https)
            })
            .unwrap_or(false);
        let preset = ProjectPreset {
            schema_version: 1,
            name,
            runtime: manifest
                .runtime
                .as_ref()
                .and_then(|record| version(&record.installation_id)),
            php_cgi: manifest
                .services
                .iter()
                .find(|service| service.id == "php-cgi")
                .and_then(|v| v.runtime.as_ref())
                .and_then(|r| version(&r.installation_id)),
            composer: manifest.tools.composer.as_deref().and_then(version),
            java: manifest.tools.java.as_deref().and_then(version),
            android_tools: manifest.tools.android_tools.as_deref().and_then(version),
            database: manifest
                .services
                .iter()
                .find(|service| matches!(service.id.as_str(), "mysql" | "postgres"))
                .map(|service| service.id.clone()),
            mail: manifest
                .services
                .iter()
                .any(|service| service.id == "mailpit"),
            https,
            tasks: manifest.tasks,
        };
        preset.validate()?;
        Ok(preset)
    }

    pub async fn check_project(&self, id: &str) -> Result<Vec<ProjectCheck>> {
        let project = self
            .projects
            .list()
            .await?
            .into_iter()
            .find(|entry| entry.manifest.id == id)
            .context("Unknown project")?;
        let manifest = project.manifest;
        let mut checks = Vec::new();
        let mut check = |code, ok, detail: String, action| {
            checks.push(ProjectCheck {
                code,
                ok,
                detail,
                action,
            })
        };
        check(
            "source",
            manifest.root.is_dir(),
            manifest.root.display().to_string(),
            "source",
        );
        let runtime = match &manifest.runtime {
            Some(binding) => {
                let result = self.runtimes.selected(&binding.installation_id).await;
                check(
                    "runtime",
                    result.is_ok(),
                    match &result {
                        Ok(record) => format!("{} {}", record.runtime, record.version),
                        Err(error) => error.to_string(),
                    },
                    "versions",
                );
                result.ok()
            }
            None => {
                check(
                    "runtime",
                    manifest.executable.is_file(),
                    manifest.executable.display().to_string(),
                    "versions",
                );
                None
            }
        };
        let tools = self.projects.tool_environment(id).await;
        check(
            "tools",
            tools.is_ok(),
            tools
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_default(),
            "versions",
        );
        if manifest.root.join("composer.json").is_file() {
            check(
                "composer",
                manifest.tools.composer.is_some() && tools.is_ok(),
                String::new(),
                "versions",
            );
            check(
                "dependencies",
                manifest.root.join("vendor/autoload.php").is_file(),
                "Composer".into(),
                "tasks",
            );
        }
        if manifest.root.join("package-lock.json").is_file() {
            check(
                "node_dependencies",
                manifest.root.join("node_modules").is_dir(),
                "npm".into(),
                "tasks",
            );
        }
        let php_cgi = manifest
            .services
            .iter()
            .find(|service| service.id == "php-cgi")
            .and_then(|service| service.runtime.as_ref());
        let php_cgi = if let Some(binding) = php_cgi {
            let selected = self.runtimes.selected(&binding.installation_id).await;
            check(
                "php_cgi",
                selected.is_ok(),
                selected
                    .as_ref()
                    .map(|record| record.version.clone())
                    .unwrap_or_else(|err| err.to_string()),
                "versions",
            );
            selected.ok()
        } else {
            None
        };
        if let Some(record) = runtime {
            if record.runtime == "flutter" && manifest.root.join("android").is_dir() {
                let env = tools.unwrap_or_default();
                let jdk = if let Some(id) = manifest.tools.java.as_deref() {
                    self.runtimes.selected(id).await.ok()
                } else {
                    None
                };
                let detail = jdk
                    .as_ref()
                    .map(|record| record.version.clone())
                    .unwrap_or_default();
                let major = detail
                    .split('.')
                    .next()
                    .and_then(|value| value.parse::<u32>().ok());
                check(
                    "jdk",
                    major.map_or_else(
                        || env.contains_key("JAVA_HOME") || std::env::var_os("JAVA_HOME").is_some(),
                        |major| major >= 17,
                    ),
                    detail,
                    "versions",
                );
                let sdk = env
                    .get("ANDROID_HOME")
                    .map(PathBuf::from)
                    .or_else(|| std::env::var_os("ANDROID_HOME").map(PathBuf::from));
                check(
                    "android",
                    sdk.is_some_and(|dir| dir.join("platforms").is_dir()),
                    String::new(),
                    "versions",
                );
            }
            if (record.runtime == "php" || php_cgi.is_some())
                && manifest.root.join("composer.json").is_file()
            {
                let root_path = manifest.root.join("composer.json");
                let mut body = Vec::new();
                std::fs::File::open(root_path)?
                    .take(262145)
                    .read_to_end(&mut body)?;
                let missing = if let Ok(config) = serde_json::from_slice::<serde_json::Value>(&body)
                {
                    let mut args = vec!["-m".into()];
                    if let Ok(env) = self.projects.tool_environment(id).await {
                        if let Some(ini) = env.get("DOCKON_COMPOSER_INI") {
                            args.splice(0..0, ["-c".into(), ini.clone()]);
                        }
                    }
                    let executable = php_cgi
                        .as_ref()
                        .map(|cgi| {
                            cgi.executable.with_file_name(if cfg!(windows) {
                                "php.exe"
                            } else {
                                "php"
                            })
                        })
                        .unwrap_or_else(|| record.executable.clone());
                    let mut command = tokio::process::Command::new(executable);
                    command.args(args).kill_on_drop(true);
                    #[cfg(windows)]
                    command.creation_flags(0x08000000);
                    match tokio::time::timeout(std::time::Duration::from_secs(4), command.output())
                        .await
                    {
                        Ok(Ok(output)) if output.status.success() => {
                            let modules =
                                String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
                            config["require"]
                                .as_object()
                                .into_iter()
                                .flat_map(|item| item.keys())
                                .filter_map(|key| key.strip_prefix("ext-"))
                                .filter(|name| {
                                    !modules.lines().any(|module| module.trim() == *name)
                                })
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                        _ => "PHP could not list extensions".into(),
                    }
                } else {
                    "composer.json could not be read".into()
                };
                check("php_extensions", missing.is_empty(), missing, "versions");
            }
        }
        Ok(checks)
    }

    pub async fn submit_task(
        self: &Arc<Self>,
        id: String,
        task: dockon_core::ProjectTask,
        device: Option<String>,
    ) -> Result<Operation> {
        let operation = uuid::Uuid::new_v4().to_string();
        self.store.begin(&operation, &id, "project.task").await?;
        let response = self.store.operation(&operation).await?;
        let state = self.clone();
        tokio::spawn(async move {
            let result = state.projects.run_project_task(&id, task, device).await;
            let error = result.err().map(|e| format!("{e:#}"));
            let _ = state.store.finish(&operation, error.as_deref()).await;
        });
        Ok(response)
    }

    pub async fn git_clone(self: &Arc<Self>, slug: String, url: String) -> Result<Operation> {
        let parsed =
            reqwest::Url::parse(url.trim()).context("Enter an HTTPS Git repository URL")?;
        if parsed.scheme() != "https"
            || parsed.host().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.port_or_known_default() != Some(443)
            || parsed.path() == "/"
        {
            bail!("Use an HTTPS repository URL without credentials, parameters or fragment");
        }
        let path = workspace::project_path(&slug)?;
        if path.exists()
            || self
                .projects
                .list()
                .await?
                .into_iter()
                .any(|p| p.manifest.id == slug)
        {
            bail!("Project folder or ID already exists");
        }
        let operation = uuid::Uuid::new_v4().to_string();
        self.store
            .begin(&operation, &slug, "project.git_clone")
            .await?;
        let result = self.store.operation(&operation).await?;
        let state = self.clone();
        tokio::spawn(async move {
            let error = clone_repository(parsed, path, &operation)
                .await
                .err()
                .map(|error| format!("{error:#}"));
            let _ = state.store.finish(&operation, error.as_deref()).await;
        });
        Ok(result)
    }
}

async fn clone_repository(url: reqwest::Url, path: PathBuf, operation: &str) -> Result<()> {
    let workspace = path.parent().context("Project workspace is missing")?;
    let stage = workspace.join(format!(".dockon-git-{operation}"));
    std::fs::create_dir(&stage)?;
    let result: Result<()> = async {
        let config = stage.join("empty-git-config");
        std::fs::File::create(&config)?;
        let source = stage.join("checkout");
        let mut git = tokio::process::Command::new(if cfg!(windows) { "git.exe" } else { "git" });
        git.args([
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "clone",
            "--no-recurse-submodules",
            "--template=",
            "--",
        ])
        .arg(url.as_str())
        .arg(&source)
        .current_dir(&stage)
        .env("GIT_CONFIG_GLOBAL", &config)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
        #[cfg(windows)]
        git.creation_flags(0x08000000);
        let output = tokio::time::timeout(std::time::Duration::from_secs(240), git.output())
            .await
            .context("Git clone timed out")?
            .context("Git is unavailable")?;
        if !output.status.success() {
            bail!(
                "Git clone failed: {}",
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(3500)
                    .collect::<String>()
            );
        }
        if path.exists() {
            bail!("Project folder was created while Git was running");
        }
        std::fs::rename(source, path)?;
        Ok(())
    }
    .await;
    let _ = std::fs::remove_dir_all(&stage);
    result
}
