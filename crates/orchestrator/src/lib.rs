mod cache;
mod local_db;
mod ports;
#[cfg(windows)]
mod smtp;
mod tasks;
mod workload;
use anyhow::{bail, Context, Result};
use dockon_core::{
    Event, EventBatch, Manifest, Operation, Project, Provider, ServiceStatus, State,
};
use dockon_persistence::Store;
use ports::PortAllocator;
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::{Mutex, RwLock};
use workload::{launch, OwnedInstance};

pub struct Events {
    seq: u64,
    epoch: String,
    entries: VecDeque<Event>,
}
impl Default for Events {
    fn default() -> Self {
        Self {
            seq: 0,
            epoch: uuid::Uuid::new_v4().to_string(),
            entries: VecDeque::new(),
        }
    }
}
pub type EventBus = Arc<Mutex<Events>>;
impl Events {
    pub fn push(&mut self, id: &str, kind: &str, message: &str) {
        self.seq += 1;
        self.entries.push_back(Event {
            seq: self.seq,
            project_id: id.into(),
            kind: kind.into(),
            message: message.chars().take(1024).collect(),
        });
        while self.entries.len() > 100 {
            self.entries.pop_front();
        }
    }
    pub fn batch(&self, after: u64) -> EventBatch {
        EventBatch {
            epoch: self.epoch.clone(),
            events: self
                .entries
                .iter()
                .filter(|e| after > self.seq || e.seq > after)
                .cloned()
                .collect(),
            truncated: self
                .entries
                .front()
                .is_some_and(|e| after != 0 && after < e.seq.saturating_sub(1)),
        }
    }
}
struct OwnedService {
    status: ServiceStatus,
    owner: String,
    provider: Provider,
    internal_port: u16,
    internal_smtp_port: Option<u16>,
    instance: OwnedInstance,
    #[cfg(windows)]
    _smtp: Option<smtp::Forward>,
}
struct Entry {
    project: RwLock<Project>,
    instance: Mutex<Option<OwnedInstance>>,
    services: Mutex<Vec<OwnedService>>,
    lifecycle: Mutex<()>,
    tasks: Mutex<Vec<tasks::TaskProcess>>,
}
impl Entry {
    fn new(manifest: Manifest) -> Self {
        Self {
            project: RwLock::new(Project {
                manifest,
                state: State::Stopped,
                pid: None,
                port: None,
                error: None,
                container_id: None,
                services: vec![],
            }),
            instance: Mutex::new(None),
            services: Mutex::new(vec![]),
            lifecycle: Mutex::new(()),
            tasks: Mutex::new(vec![]),
        }
    }
}
pub struct Orchestrator {
    entries: RwLock<BTreeMap<String, Arc<Entry>>>,
    store: Store,
    pub events: EventBus,
    closing: AtomicBool,
    data_dir: PathBuf,
    ports: PortAllocator,
    runtimes: dockon_runtime_manager::RuntimeManager,
}
impl Orchestrator {
    pub async fn new(store: Store, data_dir: PathBuf) -> Result<Self> {
        let identity = data_dir.join("installation.id");
        if !identity.is_file() {
            std::fs::write(identity, uuid::Uuid::new_v4().to_string())?;
        }
        let mut entries = BTreeMap::new();
        for manifest in store.list().await? {
            let id = manifest.id.clone();
            let entry = Arc::new(Entry::new(manifest));
            store.desired(&id, false).await?;
            entries.insert(id, entry);
        }
        Ok(Self {
            runtimes: dockon_runtime_manager::RuntimeManager::new(data_dir.clone(), store.clone())?,
            entries: RwLock::new(entries),
            store,
            events: Arc::default(),
            closing: AtomicBool::new(false),
            data_dir,
            ports: PortAllocator::default(),
        })
    }
    async fn entry(&self, id: &str) -> Result<Arc<Entry>> {
        self.entries
            .read()
            .await
            .get(id)
            .cloned()
            .context("Unknown project")
    }
    pub async fn register(&self, manifest: Manifest) -> Result<Project> {
        if manifest.provider == Provider::Container
            || manifest
                .services
                .iter()
                .any(|service| service.provider == Provider::Container)
        {
            bail!("Dockon projects use local applications and services");
        }
        manifest.validate()?;
        let mut entries = self.entries.write().await;
        if self.closing.load(Ordering::Acquire) {
            bail!("Daemon is shutting down");
        }
        if entries.contains_key(&manifest.id) {
            bail!("Project id is already registered");
        }
        self.store.register(&manifest).await?;
        let id = manifest.id.clone();
        let entry = Arc::new(Entry::new(manifest));
        let project = entry.project.read().await.clone();
        entries.insert(id, entry);
        Ok(project)
    }
    pub async fn list(&self) -> Result<Vec<Project>> {
        let entries: Vec<_> = self.entries.read().await.values().cloned().collect();
        let mut result = vec![];
        for entry in entries {
            if let Ok(_guard) = entry.lifecycle.try_lock() {
                let mut owned = entry.instance.lock().await;
                if let Some(instance) = owned.as_mut() {
                    match instance.running().await {
                        Ok(true) => {}
                        outcome => {
                            let mut project = entry.project.write().await;
                            let state = if outcome.is_err() {
                                State::Degraded
                            } else {
                                State::Failed
                            };
                            if project.state != state {
                                self.events.lock().await.push(
                                    &project.manifest.id,
                                    "state",
                                    if state == State::Failed {
                                        "FAILED: workload exited"
                                    } else {
                                        "DEGRADED: provider unavailable"
                                    },
                                );
                            }
                            project.state = state;
                            project.error = Some(match outcome {
                                Err(error) => format!("{error:#}"),
                                _ => "Managed workload exited".into(),
                            });
                            if state == State::Failed {
                                project.pid = None;
                                project.port = None;
                            }
                        }
                    }
                }
                let mut services = entry.services.lock().await;
                for service in services.iter_mut() {
                    let health = service.instance.running().await;
                    if !matches!(health, Ok(true)) {
                        service.status.state = State::Failed;
                        let mut project = entry.project.write().await;
                        if project.state == State::Running {
                            project.state = State::Degraded;
                            project.error =
                                Some(format!("Service {} unavailable", service.status.id));
                        }
                    }
                }
                entry.project.write().await.services =
                    services.iter().map(|s| s.status.clone()).collect();
            }
            result.push(entry.project.read().await.clone());
        }
        Ok(result)
    }
    pub async fn submit(self: &Arc<Self>, id: String, start: bool) -> Result<Operation> {
        let entry = self.entry(&id).await?;
        let _admission = self.entries.read().await;
        if self.closing.load(Ordering::Acquire) {
            bail!("Daemon is shutting down");
        }
        let operation = uuid::Uuid::new_v4().to_string();
        self.store
            .begin(&operation, &id, if start { "start" } else { "stop" })
            .await?;
        let reply = self.store.operation(&operation).await?;
        let state = self.clone();
        tokio::spawn(async move {
            let _guard = entry.lifecycle.lock().await;
            let result = if state.closing.load(Ordering::Acquire) {
                Err(anyhow::anyhow!(
                    "Operation canceled: daemon is shutting down"
                ))
            } else if start {
                state.start_entry(&entry).await
            } else {
                state.stop_entry(&entry).await
            };
            let mut error = result.err().map(|e| format!("{e:#}"));
            if error.is_some() && start {
                if let Err(cleanup) = state.stop_entry(&entry).await {
                    error = Some(format!(
                        "{}; cleanup failed: {cleanup:#}",
                        error.unwrap_or_default()
                    ));
                }
            }
            if let Some(message) = &error {
                if start {
                    let _ = state.store.desired(&id, false).await;
                }
                let mut project = entry.project.write().await;
                project.state = State::Failed;
                project.error = Some(message.clone());
                state.events.lock().await.push(&id, "error", message);
            }
            if let Err(error) = state.store.finish(&operation, error.as_deref()).await {
                state.events.lock().await.push(
                    &id,
                    "error",
                    &format!("Operation journal failed: {error}"),
                );
            }
        });
        Ok(reply)
    }
    pub async fn submit_static_switch(
        self: &Arc<Self>,
        id: String,
        installation_id: String,
    ) -> Result<Operation> {
        self.entry(&id).await?;
        let operation = uuid::Uuid::new_v4().to_string();
        self.store
            .begin(&operation, &id, "switch_static_server")
            .await?;
        let reply = self.store.operation(&operation).await?;
        let state = self.clone();
        tokio::spawn(async move {
            let error = state
                .switch_static_server(&id, &installation_id)
                .await
                .err()
                .map(|error| format!("{error:#}"));
            if let Err(error) = state.store.finish(&operation, error.as_deref()).await {
                state.events.lock().await.push(
                    &id,
                    "error",
                    &format!("Operation journal failed: {error}"),
                );
            }
        });
        Ok(reply)
    }
    pub async fn set_php_version(&self, id: &str, installation_id: &str) -> Result<Project> {
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        let mut project = entry.project.write().await;
        if project.state != State::Stopped || self.closing.load(Ordering::Acquire) {
            bail!("Stop the project before changing its PHP version");
        }
        let installation = self.runtimes.selected(installation_id).await?;
        if installation.runtime != "php-cgi" {
            bail!("Select a PHP FastCGI installation");
        }
        let mut manifest = project.manifest.clone();
        if let Some(service) = manifest
            .services
            .iter_mut()
            .find(|service| service.id == "php-cgi")
        {
            service.runtime = Some(dockon_core::RuntimeBinding {
                installation_id: installation.id,
                isolated_python: false,
            });
            service.executable = installation.executable;
        } else {
            let server_id = &manifest
                .runtime
                .as_ref()
                .context("Select Apache or Nginx first")?
                .installation_id;
            let server = self.runtimes.selected(server_id).await?;
            if !["apache", "nginx"].contains(&server.runtime.as_str())
                || !manifest.root.join("public/index.php").is_file()
            {
                bail!("PHP FastCGI requires an Apache or Nginx project with public/index.php");
            }
            dockon_runtime_manager::projects::attach_php_cgi(
                &self.runtimes,
                &mut manifest,
                &format!("{}:php", server.runtime),
                Some(installation_id),
            )
            .await?;
        }
        manifest.validate()?;
        self.store.update_manifest(&manifest).await?;
        project.manifest = manifest;
        Ok(project.clone())
    }

    pub async fn set_runtime_version(&self, id: &str, installation_id: &str) -> Result<Project> {
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        let mut project = entry.project.write().await;
        if project.state != State::Stopped || self.closing.load(Ordering::Acquire) {
            bail!("Stop the project before changing its runtime version");
        }
        let mut manifest = project.manifest.clone();
        let binding = manifest
            .runtime
            .as_mut()
            .context("The project has no managed runtime profile")?;
        let previous = self
            .runtimes
            .installed()
            .await?
            .into_iter()
            .find(|entry| entry.id == binding.installation_id)
            .context("Project runtime record missing")?;
        let next = self.runtimes.selected(installation_id).await?;
        if manifest.provider != Provider::Native
            || previous.runtime != next.runtime
            || ["apache", "nginx"].contains(&next.runtime.as_str())
        {
            bail!("Select another version of the project's current runtime");
        }
        if next.runtime == "dotnet" && manifest.root.join("global.json").is_file() {
            let pinned: serde_json::Value =
                serde_json::from_slice(&std::fs::read(manifest.root.join("global.json"))?)?;
            if pinned["sdk"]["version"].as_str() != Some(&next.version) {
                bail!("global.json SDK differs from the selected runtime");
            }
        }
        binding.installation_id = next.id;
        manifest.executable = next.executable;
        manifest.validate()?;
        self.store.update_manifest(&manifest).await?;
        project.manifest = manifest;
        Ok(project.clone())
    }

    pub async fn tool_environment(&self, id: &str) -> Result<BTreeMap<String, String>> {
        let entry = self.entry(id).await?;
        let manifest = entry.project.read().await.manifest.clone();
        Ok(self.runtimes.tool_environment(&manifest).await?.1)
    }

    pub async fn set_tools(&self, id: &str, tools: dockon_core::ProjectTools) -> Result<Project> {
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        let mut project = entry.project.write().await;
        if project.state != State::Stopped {
            bail!("Stop the project before changing its tools");
        }
        let mut manifest = project.manifest.clone();
        manifest.tools = tools;
        self.runtimes.tool_environment(&manifest).await?;
        self.store.update_manifest(&manifest).await?;
        project.manifest = manifest;
        Ok(project.clone())
    }

    pub async fn runtime_paths(&self, id: &str) -> Result<Vec<PathBuf>> {
        let entry = self.entry(id).await?;
        let manifest = entry.project.read().await.manifest.clone();
        let mut paths = Vec::new();
        for binding in manifest.runtime.iter().chain(
            manifest
                .services
                .iter()
                .filter_map(|service| service.runtime.as_ref()),
        ) {
            let record = self.runtimes.selected(&binding.installation_id).await?;
            if let Some(bin) = record.executable.parent() {
                if !paths.iter().any(|existing| existing == bin) {
                    paths.push(bin.to_path_buf());
                }
            }
        }
        paths.extend(self.runtimes.tool_environment(&manifest).await?.0);
        Ok(paths)
    }

    async fn switch_static_server(&self, id: &str, installation_id: &str) -> Result<Project> {
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        let current = entry.project.read().await.clone();
        if self.closing.load(Ordering::Acquire) {
            bail!("Daemon is shutting down");
        }
        if !matches!(current.state, State::Running | State::Stopped)
            || current.manifest.provider != Provider::Native
            || !current.manifest.args.is_empty()
        {
            bail!("Server switching requires a running or stopped native web project");
        }
        let binding = current
            .manifest
            .runtime
            .as_ref()
            .context("Select a static web server first")?;
        let previous = self
            .runtimes
            .installed()
            .await?
            .into_iter()
            .find(|record| record.id == binding.installation_id)
            .context("Web runtime record missing")?;
        let next = self.runtimes.selected(installation_id).await?;
        if !["apache", "nginx"].contains(&previous.runtime.as_str())
            || !["apache", "nginx"].contains(&next.runtime.as_str())
        {
            bail!("Only Apache and Nginx profiles support this switch");
        }
        if previous.id == next.id {
            return Ok(current);
        }
        let profile = if current
            .manifest
            .services
            .iter()
            .any(|service| service.id == "php-cgi")
        {
            "php"
        } else {
            "static"
        };
        let selected = dockon_runtime_manager::projects::import(
            &self.runtimes,
            id,
            &current.manifest.name,
            &current.manifest.root,
            installation_id,
            &format!("{}:{profile}", next.runtime),
        )
        .await?;
        let mut manifest = current.manifest.clone();
        manifest.executable = selected.executable;
        manifest.runtime = selected.runtime;
        if current.state == State::Stopped {
            self.store.update_manifest(&manifest).await?;
            let mut project = entry.project.write().await;
            project.manifest = manifest;
            return Ok(project.clone());
        }
        if manifest.port != 0 {
            bail!("Stop the project before switching a fixed-port web project");
        }
        let environment =
            service_environment(entry.services.lock().await.iter(), manifest.provider)?;
        let staging = self
            .data_dir
            .join("instances")
            .join(dockon_core::project_key(id))
            .join(format!("web-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&staging)?;
        let (mut candidate, port) = launch(
            &manifest,
            self.events.clone(),
            (&self.ports, &self.runtimes),
            &staging,
            id,
            "web-preview",
            &environment,
        )
        .await?;
        if let Err(error) = self.store.update_manifest(&manifest).await {
            candidate.stop(id).await?;
            return Err(error);
        }
        let mut owned = entry.instance.lock().await;
        if let Some(previous) = owned.as_mut() {
            if let Err(error) = previous.stop(id).await {
                let persistence = self.store.update_manifest(&current.manifest).await;
                let cleanup = candidate.stop(id).await;
                persistence.context("Could not restore the previous manifest")?;
                cleanup.context("Could not stop the candidate web server")?;
                return Err(error.context("Could not retire the previous web server"));
            }
        }
        let mut project = entry.project.write().await;
        project.pid = candidate.pid();
        project.port = Some(port);
        project.manifest = manifest;
        *owned = Some(candidate);
        Ok(project.clone())
    }
    pub async fn operation(&self, id: &str) -> Result<Operation> {
        self.store.operation(id).await
    }
    pub async fn add_redis(&self, _id: &str) -> Result<Project> {
        bail!("Assign a Redis version from project cache settings");
    }
    pub async fn add_mailpit(&self, id: &str) -> Result<Project> {
        self.add_service(id, Self::mail_service()?).await
    }
    pub fn mail_service() -> Result<dockon_core::ServiceSpec> {
        let executable = workload::bundled_mailpit_path()?
            .context("Local Mailpit is included in the Windows package")?;
        Ok(serde_json::from_value(serde_json::json!({
            "id": "mailpit", "executable": executable, "health": {"type":"http","path":"/readyz"}
        }))?)
    }

    pub async fn set_mail(&self, id: &str, enabled: bool) -> Result<Project> {
        if enabled {
            return self.add_mailpit(id).await;
        }
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        let mut project = entry.project.write().await;
        if project.state != State::Stopped || self.closing.load(Ordering::Acquire) {
            bail!("Stop the project before changing its mail service");
        }
        let mut manifest = project.manifest.clone();
        manifest.services.retain(|service| service.id != "mailpit");
        manifest.validate()?;
        self.store.update_manifest(&manifest).await?;
        project.manifest = manifest;
        Ok(project.clone())
    }

    pub async fn add_postgres(&self, id: &str) -> Result<Project> {
        self.add_database(id, "postgres").await
    }
    pub async fn add_database(&self, id: &str, engine: &str) -> Result<Project> {
        self.add_service(id, Self::database_service(engine)?).await
    }
    pub fn database_service(engine: &str) -> Result<dockon_core::ServiceSpec> {
        local_db::Kind::parse(engine)?.service()
    }
    pub async fn migrate_postgres(
        &self,
        id: &str,
        dockon: &std::path::Path,
        postgres: &std::path::Path,
    ) -> Result<Project> {
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        let mut project = entry.project.write().await;
        if project.state != State::Stopped || self.closing.load(Ordering::Acquire) {
            bail!("Stop the project before migrating its PostgreSQL database");
        }
        let mut manifest = project.manifest.clone();
        let service = manifest
            .services
            .iter_mut()
            .find(|service| {
                service.id == "postgres"
                    && service
                        .container
                        .as_ref()
                        .is_some_and(|spec| spec.image == dockon_core::POSTGRES_IMAGE)
            })
            .context("Project has no legacy managed PostgreSQL service")?;
        let mut replacement = Self::database_service("postgres")?;
        replacement.depends_on.clone_from(&service.depends_on);
        replacement.port = service.port;
        *service = replacement;
        manifest.validate()?;
        let data = self.service_data(id, "postgres");
        std::fs::create_dir_all(&data)?;
        if data.join("native-data").exists() {
            bail!("Local PostgreSQL data already exists; preserving both databases");
        }
        for (source, destination) in [
            (dockon, "pending-dockon.dump"),
            (postgres, "pending-postgres.dump"),
        ] {
            use std::io::Read;
            let mut input = std::fs::File::open(source)?;
            let mut header = [0; 5];
            input.read_exact(&mut header)?;
            if &header != b"PGDMP" {
                bail!("Select a PostgreSQL custom-format backup");
            }
            let temporary = data.join(format!("migration-{}.part", uuid::Uuid::new_v4()));
            use std::io::Write;
            let mut output = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            output.write_all(&header)?;
            std::io::copy(&mut input, &mut output)?;
            output.sync_all()?;
            drop(output);
            std::fs::rename(temporary, data.join(destination))?;
        }
        self.store.update_manifest(&manifest).await?;
        project.manifest = manifest;
        Ok(project.clone())
    }
    fn service_data(&self, id: &str, service: &str) -> PathBuf {
        self.data_dir
            .join("instances")
            .join(dockon_core::project_key(id))
            .join("services")
            .join(dockon_core::project_key(service))
    }
    pub async fn postgres_connection(&self, id: &str) -> Result<serde_json::Value> {
        self.database_connection(id, "postgres").await
    }
    pub async fn database_connection(&self, id: &str, engine: &str) -> Result<serde_json::Value> {
        let kind = local_db::Kind::parse(engine)?;
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        let project = entry.project.read().await;
        if !project
            .manifest
            .services
            .iter()
            .any(|service| service.id == engine && kind.matches(&service.executable))
        {
            bail!("Project has no matching local database");
        }
        let service = project
            .services
            .iter()
            .find(|service| service.id == engine && service.state == State::Running)
            .context("Start the project's database first")?;
        let password = std::fs::read_to_string(kind.password_file(&self.service_data(id, engine)))?;
        Ok(serde_json::json!({
            "host": "127.0.0.1",
            "port": service.port,
            "user": kind.user(),
            "database": "dockon",
            "password": password
        }))
    }
    pub async fn backup_postgres(&self, id: &str) -> Result<PathBuf> {
        self.snapshot(id, "postgres").await
    }

    pub async fn snapshot(&self, id: &str, engine: &str) -> Result<PathBuf> {
        let kind = local_db::Kind::parse(engine)?;
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        let project = entry.project.read().await;
        if !project
            .manifest
            .services
            .iter()
            .any(|service| service.id == engine && kind.matches(&service.executable))
        {
            bail!("Project has no matching local database");
        }
        drop(project);
        let services = entry.services.lock().await;
        let service = services
            .iter()
            .find(|service| service.status.id == engine)
            .context("Start the database first")?;
        let dump = service
            .instance
            .database()
            .context("Start the local database service")?
            .dump()
            .await?;
        if dump.len() > 8 * 1024 * 1024 {
            bail!("Database backup exceeds 8 MiB");
        }
        let directory = self
            .data_dir
            .join("backups")
            .join(dockon_core::project_key(id));
        std::fs::create_dir_all(&directory)?;
        let name = format!("{engine}-{}", uuid::Uuid::new_v4());
        let partial = directory.join(format!("{name}.partial"));
        let destination = directory.join(format!("{name}.sql"));
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        file.write_all(&dump)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&partial, &destination)?;
        let digest = dockon_runtime_manager::hash_file(&destination, "sha256")?;
        std::fs::write(destination.with_extension("sha256"), digest)?;
        Ok(destination)
    }
    pub async fn postgres_backups(&self, id: &str) -> Result<Vec<String>> {
        self.snapshots(id, "postgres").await
    }
    pub async fn snapshots(&self, id: &str, engine: &str) -> Result<Vec<String>> {
        local_db::Kind::parse(engine)?;
        let prefix = format!("{engine}-");
        self.entry(id).await?;
        let directory = self
            .data_dir
            .join("backups")
            .join(dockon_core::project_key(id));
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut backups = Vec::new();
        for entry in entries {
            let entry = entry?;
            let Some(name) = entry.file_name().into_string().ok() else {
                continue;
            };
            let uuid = name
                .strip_prefix(&prefix)
                .and_then(|name| name.strip_suffix(".sql"));
            if uuid.is_some_and(|uuid| uuid::Uuid::parse_str(uuid).is_ok())
                && entry.file_type()?.is_file()
                && entry.path().with_extension("sha256").is_file()
            {
                backups.push(name);
            }
        }
        backups.sort_unstable();
        Ok(backups)
    }
    pub async fn submit_postgres_restore(
        self: &Arc<Self>,
        id: String,
        backup: String,
    ) -> Result<Operation> {
        self.submit_snapshot_copy(id.clone(), "postgres".into(), backup, id)
            .await
    }
    pub async fn submit_snapshot(
        self: &Arc<Self>,
        id: String,
        engine: String,
    ) -> Result<Operation> {
        local_db::Kind::parse(&engine)?;
        self.entry(&id).await?;
        let operation = uuid::Uuid::new_v4().to_string();
        self.store
            .begin(&operation, &id, "database.snapshot")
            .await?;
        let reply = self.store.operation(&operation).await?;
        let state = self.clone();
        tokio::spawn(async move {
            let error = state
                .snapshot(&id, &engine)
                .await
                .err()
                .map(|e| format!("{e:#}"));
            let _ = state.store.finish(&operation, error.as_deref()).await;
        });
        Ok(reply)
    }
    pub async fn submit_snapshot_copy(
        self: &Arc<Self>,
        id: String,
        engine: String,
        backup: String,
        target: String,
    ) -> Result<Operation> {
        local_db::Kind::parse(&engine)?;
        self.entry(&target).await?;
        self.entry(&id).await?;
        let operation = uuid::Uuid::new_v4().to_string();
        self.store
            .begin(&operation, &target, "database.restore_copy")
            .await?;
        let reply = self.store.operation(&operation).await?;
        let state = self.clone();
        tokio::spawn(async move {
            let error = state
                .restore_snapshot(&id, &engine, &backup, &target, &operation)
                .await
                .err()
                .map(|error| error.to_string());
            let _ = state.store.finish(&operation, error.as_deref()).await;
        });
        Ok(reply)
    }
    async fn restore_snapshot(
        &self,
        id: &str,
        engine: &str,
        backup: &str,
        target: &str,
        operation: &str,
    ) -> Result<()> {
        let entry = self.entry(target).await?;
        let kind = local_db::Kind::parse(engine)?;
        let prefix = format!("{engine}-");
        let _guard = entry.lifecycle.lock().await;
        if self.closing.load(Ordering::Acquire) {
            bail!("Daemon is shutting down");
        }
        let name = backup
            .strip_prefix(&prefix)
            .and_then(|name| name.strip_suffix(".sql"))
            .context("Choose a Dockon database snapshot")?;
        let uuid = uuid::Uuid::parse_str(name)?;
        let path = self
            .data_dir
            .join("backups")
            .join(dockon_core::project_key(id))
            .join(format!("{engine}-{uuid}.sql"));
        let expected = std::fs::read_to_string(path.with_extension("sha256"))?;
        use std::io::Read;
        let mut sql = Vec::new();
        std::fs::File::open(&path)?
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut sql)?;
        use sha2::{Digest, Sha256};
        if sql.len() > 8 * 1024 * 1024 || format!("{:x}", Sha256::digest(&sql)) != expected {
            bail!("Backup size or integrity check failed");
        }
        let project = entry.project.read().await;
        if !project
            .manifest
            .services
            .iter()
            .any(|service| service.id == engine && kind.matches(&service.executable))
        {
            bail!("Target has no matching local database service");
        }
        drop(project);
        let services = entry.services.lock().await;
        let service = services
            .iter()
            .find(|service| service.status.id == engine)
            .context("Start the target database first")?;
        let process = service
            .instance
            .database()
            .context("Start the target database service")?;
        let database = format!(
            "dockon_restore_{}",
            uuid::Uuid::parse_str(operation)?.simple()
        );
        process.restore_copy(&sql, &database).await?;
        self.events.lock().await.push(
            target,
            "backup",
            &format!("Restored database copy {database}"),
        );
        Ok(())
    }
    async fn add_service(&self, id: &str, service: dockon_core::ServiceSpec) -> Result<Project> {
        if service.provider == Provider::Container {
            bail!("Choose a local service");
        }
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        if self.closing.load(Ordering::Acquire) {
            bail!("Daemon is shutting down");
        }
        let mut project = entry.project.write().await;
        if project.state != State::Stopped {
            bail!("Stop the project before changing its service dependencies");
        }
        if project
            .manifest
            .services
            .iter()
            .any(|existing| existing.id == service.id)
        {
            bail!("This project already has a service named {}", service.id);
        }
        let mut manifest = project.manifest.clone();
        if manifest.provider == Provider::Container
            && manifest
                .container
                .as_ref()
                .map(|container| container.engine)
                != service.container.as_ref().map(|container| container.engine)
        {
            bail!("The project and service must use the same container engine");
        }
        manifest.services.push(service);
        manifest.validate()?;
        self.store.update_manifest(&manifest).await?;
        project.manifest = manifest;
        Ok(project.clone())
    }
    async fn start_entry(&self, entry: &Entry) -> Result<()> {
        {
            let project = entry.project.read().await;
            if project.manifest.provider == Provider::Container
                || project
                    .manifest
                    .services
                    .iter()
                    .any(|service| service.provider == Provider::Container)
            {
                bail!("This project has a legacy container assignment. Choose a local service before starting it.");
            }
        }
        {
            let mut owned = entry.instance.lock().await;
            if let Some(instance) = owned.as_mut() {
                if instance.running().await? {
                    return Ok(());
                }
                let id = entry.project.read().await.manifest.id.clone();
                instance.stop(&id).await?;
                *owned = None;
            }
        }
        tasks::stop_all(entry).await?;
        self.stop_services(entry).await?;
        let manifest = {
            let mut project = entry.project.write().await;
            project.state = State::Starting;
            project.pid = None;
            project.port = None;
            project.container_id = None;
            project.error = None;
            project.manifest.clone()
        };
        // Validate all service paths/DAG before creating any process or resource.
        manifest.validate()?;
        self.store.desired(&manifest.id, true).await?;
        self.events
            .lock()
            .await
            .push(&manifest.id, "state", "STARTING");
        let data = self
            .data_dir
            .join("instances")
            .join(dockon_core::project_key(&manifest.id));
        std::fs::create_dir_all(&data)?;
        {
            let mut services = entry.services.lock().await;
            for service in manifest.service_order()? {
                self.events.lock().await.push(
                    &manifest.id,
                    "service",
                    &format!("Starting service {}", service.id),
                );
                let child = service.manifest(&manifest)?;
                let child_data = data
                    .join("services")
                    .join(dockon_core::project_key(&service.id));
                std::fs::create_dir_all(&child_data)?;
                let selected: Vec<_> = services
                    .iter()
                    .filter(|s| service.depends_on.contains(&s.status.id))
                    .collect();
                let environment = service_environment(selected.into_iter(), service.provider)?;
                let (instance, port) = launch(
                    &child,
                    self.events.clone(),
                    (&self.ports, &self.runtimes),
                    &child_data,
                    &manifest.id,
                    &service.id,
                    &environment,
                )
                .await?;
                let status = ServiceStatus {
                    id: service.id.clone(),
                    state: State::Running,
                    pid: instance.pid(),
                    port,
                    container_id: None,
                    smtp_port: instance.smtp_port(),
                };
                services.push(OwnedService {
                    status,
                    owner: child.id,
                    provider: service.provider,
                    internal_port: service.container.as_ref().map_or(port, |c| c.internal_port),
                    internal_smtp_port: service
                        .container
                        .as_ref()
                        .and_then(|container| container.smtp_port),
                    instance,
                    #[cfg(windows)]
                    _smtp: None,
                });
                #[cfg(windows)]
                if service.id == "mailpit" {
                    if let Some(owned) = services.last_mut() {
                        if let Some(port) = owned.status.smtp_port {
                            owned._smtp = Some(smtp::Forward::start(&manifest.id, port).await?);
                        }
                    }
                }
                entry.project.write().await.services =
                    services.iter().map(|s| s.status.clone()).collect();
                self.events.lock().await.push(
                    &manifest.id,
                    "service",
                    &format!("Service {} ready", service.id),
                );
            }
        }
        let environment =
            service_environment(entry.services.lock().await.iter(), manifest.provider)?;
        let app_data = data.join("app");
        std::fs::create_dir_all(&app_data)?;
        let (instance, port) = launch(
            &manifest,
            self.events.clone(),
            (&self.ports, &self.runtimes),
            &app_data,
            &manifest.id,
            "app",
            &environment,
        )
        .await?;
        let mut project = entry.project.write().await;
        project.pid = instance.pid();
        project.container_id = None;
        project.port = Some(port);
        project.state = State::Running;
        *entry.instance.lock().await = Some(instance);
        self.events
            .lock()
            .await
            .push(&manifest.id, "state", "RUNNING");
        Ok(())
    }
    async fn stop_services(&self, entry: &Entry) -> Result<()> {
        let mut services = entry.services.lock().await;
        let mut errors = vec![];
        for service in services.iter_mut().rev() {
            if let Err(error) = service.instance.stop(&service.owner).await {
                errors.push(format!("{}: {error:#}", service.status.id));
            } else {
                service.status.state = State::Stopped;
            }
        }
        services.retain(|s| s.status.state != State::Stopped);
        entry.project.write().await.services = services.iter().map(|s| s.status.clone()).collect();
        if !errors.is_empty() {
            bail!("Service cleanup failed: {}", errors.join("; "));
        }
        Ok(())
    }
    async fn stop_entry(&self, entry: &Entry) -> Result<()> {
        let id = entry.project.read().await.manifest.id.clone();
        self.store.desired(&id, false).await?;
        entry.project.write().await.state = State::Stopping;
        #[cfg(windows)]
        if entry.project.read().await.manifest.provider == Provider::Native {
            self.events.lock().await.push(&id,"warning","Windows native stop terminates the owned Job Object; graceful signals are unavailable for this launch profile");
        }
        self.events.lock().await.push(&id, "state", "STOPPING");
        {
            let mut owned = entry.instance.lock().await;
            if let Some(instance) = owned.as_mut() {
                instance.stop(&id).await?;
                *owned = None;
            }
        }
        tasks::stop_all(entry).await?;
        self.stop_services(entry).await?;
        let mut project = entry.project.write().await;
        project.state = State::Stopped;
        project.pid = None;
        project.port = None;
        project.container_id = None;
        project.error = None;
        self.events.lock().await.push(&id, "state", "STOPPED");
        Ok(())
    }
    pub async fn shutdown(&self) -> Result<()> {
        let entries: Vec<_> = {
            let entries = self.entries.write().await;
            self.closing.store(true, Ordering::Release);
            entries.values().cloned().collect()
        };
        let mut errors = vec![];
        for entry in entries {
            let _guard = entry.lifecycle.lock().await;
            if let Err(error) = self.stop_entry(&entry).await {
                errors.push(format!("{error:#}"));
            }
        }
        if !errors.is_empty() {
            bail!("Shutdown cleanup failed: {}", errors.join("; "));
        }
        Ok(())
    }
}

fn service_environment<'a>(
    services: impl Iterator<Item = &'a OwnedService>,
    consumer: Provider,
) -> Result<BTreeMap<String, String>> {
    let mut endpoints = BTreeMap::new();
    let mut environment = BTreeMap::new();
    for service in services {
        if consumer == Provider::Container && service.provider == Provider::Native {
            bail!("Container-to-native service routing is not implemented; choose a native application or container dependency");
        }
        let host = if consumer == Provider::Container {
            service.status.id.as_str()
        } else {
            "127.0.0.1"
        };
        let port = if consumer == Provider::Container {
            service.internal_port
        } else {
            service.status.port
        };
        let smtp = if consumer == Provider::Container {
            service.internal_smtp_port
        } else {
            service.status.smtp_port
        };
        endpoints.insert(
            service.status.id.clone(),
            serde_json::json!({
                "host": host,
                "port": port,
                "smtp_port": smtp
            }),
        );
        if service.status.id == "mailpit" {
            if let Some(smtp) = smtp {
                environment.insert("SMTP_HOST".into(), host.into());
                environment.insert("SMTP_PORT".into(), smtp.to_string());
            }
        }
        if service.status.id == "redis" {
            environment.insert("REDIS_URL".into(), format!("redis://{host}:{port}"));
        }
        if service.status.id == "memcached" {
            environment.insert("MEMCACHED_HOST".into(), host.into());
            environment.insert("MEMCACHED_PORT".into(), port.to_string());
        }
    }
    environment.insert("DOCKON_SERVICES".into(), serde_json::to_string(&endpoints)?);
    Ok(environment)
}
