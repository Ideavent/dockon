use super::{workload, Entry, Orchestrator};
use anyhow::{bail, Context, Result};
use dockon_core::{HealthCheck, Manifest, ProjectTask, State};
use dockon_provider_native::Instance;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
};

pub(super) struct TaskProcess {
    kind: ProjectTask,
    instance: Option<Instance>,
    exit_code: Option<i32>,
    _port: Option<super::ports::PortLease>,
}

#[derive(Serialize)]
pub struct TaskStatus {
    pub task: ProjectTask,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub port: Option<u16>,
}

#[derive(Serialize)]
pub struct FlutterTarget {
    id: String,
    name: String,
    emulator: bool,
}

impl TaskProcess {
    fn status(&mut self) -> Result<TaskStatus> {
        if let Some(instance) = &mut self.instance {
            if let Some(exit) = instance.child.try_wait()? {
                self.exit_code = Some(exit.code().unwrap_or(-1));
                self.instance = None;
                self._port = None;
            }
        }
        Ok(TaskStatus {
            task: self.kind,
            running: self.instance.is_some(),
            exit_code: self.exit_code,
            port: self._port.as_ref().map(|port| port.port),
        })
    }

    async fn stop(&mut self) -> Result<()> {
        if let Some(instance) = &mut self.instance {
            let owner = instance.owner.clone();
            if instance.child.try_wait()?.is_none() {
                instance.stop(&owner).await?;
            }
        }
        self.instance = None;
        self._port = None;
        self.exit_code = Some(-1);
        Ok(())
    }
}

pub(super) async fn stop_all(entry: &Entry) -> Result<()> {
    let mut tasks = entry.tasks.lock().await;
    let mut errors = Vec::new();
    for task in tasks.iter_mut().filter(|task| task.instance.is_some()) {
        if let Err(error) = task.stop().await {
            errors.push(error.to_string());
        }
    }
    if !errors.is_empty() {
        bail!("Task cleanup failed: {}", errors.join(", "));
    }
    Ok(())
}

fn npm_cli(executable: &Path, root: &Path) -> Result<PathBuf> {
    let bin = executable.parent().context("Node CLI directory missing")?;
    [
        bin.join("node_modules/npm/bin/npm-cli.js"),
        root.join("lib/node_modules/npm/bin/npm-cli.js"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .context("Select a Node.js installation with npm")
}

fn device_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

impl Orchestrator {
    pub async fn project_tasks(&self, id: &str) -> Result<Vec<TaskStatus>> {
        let entry = self.entry(id).await?;
        let mut tasks = entry.tasks.lock().await;
        tasks.iter_mut().map(TaskProcess::status).collect()
    }

    pub async fn set_project_tasks(&self, id: &str, tasks: Vec<ProjectTask>) -> Result<()> {
        if tasks.len() > 4
            || tasks.iter().any(|task| {
                matches!(
                    task,
                    ProjectTask::FlutterDevice | ProjectTask::FlutterEmulator
                )
            })
            || tasks
                .iter()
                .enumerate()
                .any(|(i, task)| tasks[..i].contains(task))
        {
            bail!("Invalid project task selection");
        }
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        let mut project = entry.project.write().await;
        let mut manifest = project.manifest.clone();
        manifest.tasks = tasks;
        self.store.update_manifest(&manifest).await?;
        project.manifest = manifest;
        Ok(())
    }

    pub async fn stop_project_task(&self, id: &str, task: ProjectTask) -> Result<()> {
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        if let Some(task) = entry
            .tasks
            .lock()
            .await
            .iter_mut()
            .find(|entry| entry.kind == task && entry.instance.is_some())
        {
            task.stop().await?;
        }
        Ok(())
    }

    pub async fn run_project_task(
        &self,
        id: &str,
        task: ProjectTask,
        device: Option<String>,
    ) -> Result<TaskStatus> {
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        if self.closing.load(std::sync::atomic::Ordering::Acquire) {
            bail!("Daemon is shutting down");
        }
        let project = entry.project.read().await.clone();
        let needs_app = matches!(task, ProjectTask::LaravelQueue | ProjectTask::NodeDev);
        if needs_app && !matches!(project.state, State::Running | State::Degraded) {
            bail!("Start the project before running this task");
        }
        let mut processes = entry.tasks.lock().await;
        if let Some(old) = processes.iter_mut().find(|entry| entry.kind == task) {
            if old.status()?.running {
                bail!("This task is already running");
            }
        }
        let mut manifest = project.manifest;
        let binding = manifest
            .runtime
            .as_ref()
            .context("Select a project runtime first")?;
        let runtime = self.runtimes.selected(&binding.installation_id).await?;
        let mut paths = self.runtime_paths(id).await?;
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let mut env = self.tool_environment(id).await?;
        env.insert(
            "PATH".into(),
            std::env::join_paths(paths)?.to_string_lossy().into(),
        );
        env.extend(super::service_environment(
            entry.services.lock().await.iter(),
            manifest.provider,
        )?);
        let app_data = self
            .data_dir
            .join("instances")
            .join(dockon_core::project_key(id))
            .join("app");
        let task_data = app_data.join("tasks");
        std::fs::create_dir_all(&task_data)?;
        let lease = if task == ProjectTask::NodeDev {
            Some(self.ports.reserve(0)?)
        } else {
            None
        };
        let port = lease.as_ref().map(|lease| lease.port).unwrap_or(0);
        if matches!(runtime.runtime.as_str(), "python" | "flutter")
            || (runtime.runtime == "java" && manifest.template.as_deref() == Some("spring-boot"))
        {
            (manifest, env) = self
                .runtimes
                .prepare(&manifest, &app_data, &env, port)
                .await?;
        }
        if !env.contains_key("DOCKON_COMPOSER_INI") {
            let ini = if runtime.runtime == "php" {
                app_data.join("php.ini")
            } else {
                self.service_data(id, "php-cgi").join("php.ini")
            };
            if ini.is_file() {
                env.insert("DOCKON_COMPOSER_INI".into(), ini.to_string_lossy().into());
            }
        }
        let mut args = Vec::<String>::new();
        match task {
            ProjectTask::LaravelQueue => {
                if !manifest.root.join("artisan").is_file() {
                    bail!("This project does not have Laravel Artisan");
                }
                select_php(
                    &mut manifest,
                    &env,
                    &runtime.executable,
                    &runtime.runtime,
                    &mut args,
                )?;
                args.extend(["artisan".into(), "queue:work".into(), "--sleep=2".into()]);
            }
            ProjectTask::NodeDev => {
                if runtime.runtime != "node"
                    || !manifest
                        .root
                        .join("node_modules/vite/bin/vite.js")
                        .is_file()
                {
                    bail!("Install the Vite dependencies before running the development server");
                }
                args.extend([
                    "node_modules/vite/bin/vite.js".into(),
                    "--host=127.0.0.1".into(),
                    "--strictPort".into(),
                    format!("--port={port}"),
                ]);
            }
            ProjectTask::Dependencies => {
                if manifest.root.join("composer.json").is_file() {
                    let composer = env
                        .get("DOCKON_COMPOSER")
                        .context("Assign Composer in project settings")?;
                    select_php(
                        &mut manifest,
                        &env,
                        &runtime.executable,
                        &runtime.runtime,
                        &mut args,
                    )?;
                    args.extend([
                        composer.clone(),
                        "install".into(),
                        "--no-interaction".into(),
                        "--prefer-dist".into(),
                    ]);
                } else if runtime.runtime == "node" && manifest.root.join("package.json").is_file()
                {
                    let operation = if manifest.root.join("package-lock.json").is_file() {
                        "ci"
                    } else {
                        "install"
                    };
                    args.extend([
                        npm_cli(&runtime.executable, &runtime.root)?
                            .to_string_lossy()
                            .into(),
                        operation.into(),
                        "--no-fund".into(),
                    ]);
                } else if runtime.runtime == "python"
                    && manifest.root.join("requirements.txt").is_file()
                {
                    args.extend([
                        "-m".into(),
                        "pip".into(),
                        "install".into(),
                        "--no-input".into(),
                        "-r".into(),
                        "requirements.txt".into(),
                    ]);
                } else if runtime.runtime == "python"
                    && manifest.root.join("pyproject.toml").is_file()
                {
                    args.extend([
                        "-m".into(),
                        "pip".into(),
                        "install".into(),
                        "--no-input".into(),
                        "-e".into(),
                        ".".into(),
                    ]);
                } else if runtime.runtime == "flutter" {
                    args.extend(["pub".into(), "get".into()]);
                } else {
                    bail!("No supported dependency file was found for the selected tools");
                }
            }
            ProjectTask::Tests => match runtime.runtime.as_str() {
                "node" => args.extend([
                    npm_cli(&runtime.executable, &runtime.root)?
                        .to_string_lossy()
                        .into(),
                    "test".into(),
                ]),
                "python" if manifest.root.join("manage.py").is_file() => {
                    args.extend(["manage.py".into(), "test".into()])
                }
                "python" => args.extend(["-m".into(), "pytest".into()]),
                "dotnet" => args.push("test".into()),
                "go" => args.extend(["test".into(), "./...".into()]),
                "rust" | "flutter" => args.push("test".into()),
                "java" if manifest.template.as_deref() == Some("spring-boot") => {
                    args.push("test".into())
                }
                _ if manifest.root.join("artisan").is_file() => {
                    select_php(
                        &mut manifest,
                        &env,
                        &runtime.executable,
                        &runtime.runtime,
                        &mut args,
                    )?;
                    args.extend(["artisan".into(), "test".into()]);
                }
                _ => bail!("No supported test command for this project"),
            },
            ProjectTask::FlutterEmulator => {
                if runtime.runtime != "flutter" {
                    bail!("Select Flutter SDK for this project");
                }
                let target = device
                    .as_deref()
                    .filter(|id| device_id(id))
                    .context("Select an installed Android emulator")?;
                let emulator = android_emulator(&env)?;
                if !list_avds(&emulator, &env)
                    .await?
                    .iter()
                    .any(|id| id == target)
                {
                    bail!("The emulator is unavailable");
                }
                manifest.executable = emulator;
                args.extend(["-avd".into(), target.into()]);
            }
            ProjectTask::FlutterDevice => {
                if runtime.runtime != "flutter" {
                    bail!("Select Flutter SDK for this project");
                }
                let target = device
                    .as_deref()
                    .filter(|id| device_id(id))
                    .context("Select a Flutter device")?;
                if !detect_flutter_targets(&manifest, &env)
                    .await?
                    .iter()
                    .any(|device| device.id == target)
                {
                    bail!("The Flutter device is no longer connected");
                }
                args.extend(["run".into(), "-d".into(), target.into()]);
            }
        }
        manifest.args = args.into_iter().map(dockon_core::LaunchArg::from).collect();
        manifest.services.clear();
        manifest.health = HealthCheck::Tcp;
        let mut instance = Instance::spawn(&manifest, port, &task_data, &env)?;
        let label = format!("task:{task:?}");
        if let Some(stdout) = instance.child.stdout.take() {
            workload::capture(
                stdout,
                id.into(),
                label.clone(),
                "stdout",
                self.events.clone(),
            );
        }
        if let Some(stderr) = instance.child.stderr.take() {
            workload::capture(stderr, id.into(), label, "stderr", self.events.clone());
        }
        processes.retain(|entry| entry.kind != task);
        let mut process = TaskProcess {
            kind: task,
            instance: Some(instance),
            exit_code: None,
            _port: lease,
        };
        let status = process.status()?;
        processes.push(process);
        Ok(status)
    }

    pub async fn flutter_emulators(&self, id: &str) -> Result<Vec<String>> {
        let entry = self.entry(id).await?;
        let manifest = entry.project.read().await.manifest.clone();
        let binding = manifest
            .runtime
            .as_ref()
            .context("Select Flutter SDK first")?;
        if self
            .runtimes
            .selected(&binding.installation_id)
            .await?
            .runtime
            != "flutter"
        {
            bail!("Select Flutter SDK first");
        }
        let env = self.runtimes.tool_environment(&manifest).await?.1;
        let executable = android_emulator(&env)?;
        list_avds(&executable, &env).await
    }
    pub async fn flutter_targets(&self, id: &str) -> Result<Vec<FlutterTarget>> {
        let entry = self.entry(id).await?;
        let manifest = entry.project.read().await.manifest.clone();
        let binding = manifest
            .runtime
            .as_ref()
            .context("Select Flutter SDK first")?;
        if self
            .runtimes
            .selected(&binding.installation_id)
            .await?
            .runtime
            != "flutter"
        {
            bail!("Select Flutter SDK first");
        }
        let data = self
            .data_dir
            .join("instances")
            .join(dockon_core::project_key(id))
            .join("app");
        let (manifest, env) = self
            .runtimes
            .prepare(&manifest, &data, &BTreeMap::new(), 0)
            .await?;
        detect_flutter_targets(&manifest, &env).await
    }
}

fn select_php(
    manifest: &mut Manifest,
    env: &BTreeMap<String, String>,
    executable: &Path,
    runtime: &str,
    args: &mut Vec<String>,
) -> Result<()> {
    manifest.executable = if let Some(path) = env.get("DOCKON_COMPOSER_PHP") {
        PathBuf::from(path)
    } else if runtime == "php" {
        executable.to_path_buf()
    } else {
        manifest
            .services
            .iter()
            .find(|service| service.id == "php-cgi")
            .context("Select PHP for the project")?
            .executable
            .with_file_name(if cfg!(windows) { "php.exe" } else { "php" })
    };
    if let Some(ini) = env.get("DOCKON_COMPOSER_INI") {
        args.extend(["-c".into(), ini.clone()]);
    }
    Ok(())
}

fn android_emulator(env: &BTreeMap<String, String>) -> Result<PathBuf> {
    let sdk = env
        .get("ANDROID_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("ANDROID_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("ANDROID_SDK_ROOT").map(PathBuf::from))
        .context("Select an Android SDK in project settings")?;
    let executable = sdk.join(if cfg!(windows) {
        "emulator/emulator.exe"
    } else {
        "emulator/emulator"
    });
    if !executable.is_file() {
        bail!("Install the Android emulator package into this SDK");
    }
    Ok(executable)
}

async fn list_avds(executable: &Path, env: &BTreeMap<String, String>) -> Result<Vec<String>> {
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("-list-avds")
        .envs(env)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output =
        tokio::time::timeout(std::time::Duration::from_secs(10), command.output()).await??;
    if !output.status.success() {
        bail!("Android emulator could not list virtual devices");
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|id| device_id(id))
        .map(str::to_owned)
        .collect())
}

async fn detect_flutter_targets(
    manifest: &Manifest,
    env: &BTreeMap<String, String>,
) -> Result<Vec<FlutterTarget>> {
    let mut command = tokio::process::Command::new(&manifest.executable);
    command
        .args(["devices", "--machine"])
        .current_dir(&manifest.root)
        .envs(env)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output =
        tokio::time::timeout(std::time::Duration::from_secs(18), command.output()).await??;
    if !output.status.success() {
        bail!(
            "Flutter could not list devices: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(1400)
                .collect::<String>()
        );
    }
    let text = std::str::from_utf8(&output.stdout)?;
    let start = text
        .find('[')
        .context("Flutter did not report a device list")?;
    let end = text
        .rfind(']')
        .context("Flutter returned an incomplete device list")?;
    let json: Vec<serde_json::Value> = serde_json::from_str(&text[start..=end])?;
    Ok(json
        .iter()
        .filter_map(|device| {
            let id = device["id"].as_str()?;
            let platform = device["targetPlatform"].as_str()?;
            if !device_id(id)
                || !platform.starts_with("android")
                || device["isSupported"].as_bool() == Some(false)
            {
                return None;
            }
            Some(FlutterTarget {
                id: id.into(),
                name: device["name"].as_str().unwrap_or(id).into(),
                emulator: device["emulator"].as_bool().unwrap_or(false),
            })
        })
        .collect())
}
