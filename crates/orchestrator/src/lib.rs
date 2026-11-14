mod ports;
mod workload;
use anyhow::{bail, Context, Result};
use dockon_core::{
    Event, EventBatch, Manifest, Operation, Project, Provider, ServiceStatus, State,
};
use dockon_persistence::Store;
use dockon_provider_container::ContainerProvider;
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
    instance: OwnedInstance,
}
struct Entry {
    project: RwLock<Project>,
    instance: Mutex<Option<OwnedInstance>>,
    services: Mutex<Vec<OwnedService>>,
    lifecycle: Mutex<()>,
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
        }
    }
}
pub struct Orchestrator {
    entries: RwLock<BTreeMap<String, Arc<Entry>>>,
    store: Store,
    pub events: EventBus,
    closing: AtomicBool,
    containers: ContainerProvider,
    data_dir: PathBuf,
    ports: PortAllocator,
    runtimes: dockon_runtime_manager::RuntimeManager,
}
impl Orchestrator {
    pub async fn new(store: Store, data_dir: PathBuf) -> Result<Self> {
        let identity = data_dir.join("installation.id");
        let installation = if identity.is_file() {
            std::fs::read_to_string(identity)?
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            std::fs::write(identity, &id)?;
            id
        };
        let containers = ContainerProvider::new(installation)?;
        let mut entries = BTreeMap::new();
        for manifest in store.list().await? {
            let mut engines = vec![];
            if let Some(spec) = &manifest.container {
                engines.push(spec.engine);
            }
            for service in &manifest.services {
                if let Some(spec) = &service.container {
                    if !engines.contains(&spec.engine) {
                        engines.push(spec.engine);
                    }
                }
            }
            let mut recovery_error = None;
            for engine in engines {
                if let Err(error) = containers.reconcile(engine, &manifest.id).await {
                    recovery_error = Some(format!("Recovery requires provider: {error:#}"));
                }
            }
            let id = manifest.id.clone();
            let entry = Arc::new(Entry::new(manifest));
            if let Some(error) = recovery_error {
                let mut project = entry.project.write().await;
                project.state = State::Failed;
                project.error = Some(error);
            } else {
                store.desired(&id, false).await?;
            }
            entries.insert(id, entry);
        }
        Ok(Self {
            runtimes: dockon_runtime_manager::RuntimeManager::new(data_dir.clone(), store.clone())?,
            entries: RwLock::new(entries),
            store,
            events: Arc::default(),
            closing: AtomicBool::new(false),
            containers,
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
    async fn switch_static_server(&self, id: &str, installation_id: &str) -> Result<Project> {
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        let current = entry.project.read().await.clone();
        if self.closing.load(Ordering::Acquire) {
            bail!("Daemon is shutting down");
        }
        if !matches!(current.state, State::Running | State::Stopped)
            || current.manifest.provider != Provider::Native
            || !current.manifest.services.is_empty()
            || !current.manifest.args.is_empty()
        {
            bail!("Server switching requires a running or stopped native static site without services");
        }
        let binding = current
            .manifest
            .runtime
            .as_ref()
            .context("Select a static web server first")?;
        let previous = self.runtimes.selected(&binding.installation_id).await?;
        let next = self.runtimes.selected(installation_id).await?;
        if !["apache", "nginx"].contains(&previous.runtime.as_str())
            || !["apache", "nginx"].contains(&next.runtime.as_str())
        {
            bail!("Only Apache and Nginx static profiles support this switch");
        }
        if previous.id == next.id {
            return Ok(current);
        }
        let selected = dockon_runtime_manager::projects::import(
            &self.runtimes,
            id,
            &current.manifest.name,
            &current.manifest.root,
            installation_id,
            &format!("{}:static", next.runtime),
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
            bail!("Stop the project before switching a fixed-port static site");
        }
        let staging = self
            .data_dir
            .join("instances")
            .join(dockon_core::project_key(id))
            .join(format!("web-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&staging)?;
        let (mut candidate, port) = launch(
            &manifest,
            self.events.clone(),
            (&self.containers, &self.ports, &self.runtimes),
            &staging,
            id,
            "web-preview",
            &BTreeMap::new(),
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
    async fn start_entry(&self, entry: &Entry) -> Result<()> {
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
                    (&self.containers, &self.ports, &self.runtimes),
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
                    container_id: instance.container_id(),
                };
                services.push(OwnedService {
                    status,
                    owner: child.id,
                    provider: service.provider,
                    internal_port: service.container.as_ref().map_or(port, |c| c.internal_port),
                    instance,
                });
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
            (&self.containers, &self.ports, &self.runtimes),
            &app_data,
            &manifest.id,
            "app",
            &environment,
        )
        .await?;
        let mut project = entry.project.write().await;
        project.pid = instance.pid();
        project.container_id = instance.container_id();
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
    for service in services {
        if consumer == Provider::Container && service.provider == Provider::Native {
            bail!("Container-to-native service routing is not implemented; choose a native application or container dependency");
        }
        endpoints.insert(service.status.id.clone(),serde_json::json!({"host":if consumer==Provider::Container{service.status.id.as_str()}else{"127.0.0.1"},"port":if consumer==Provider::Container{service.internal_port}else{service.status.port}}));
    }
    Ok(BTreeMap::from([(
        "DOCKON_SERVICES".into(),
        serde_json::to_string(&endpoints)?,
    )]))
}
