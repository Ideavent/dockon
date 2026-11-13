use anyhow::{bail, Context, Result};
use dockon_core::{Event, EventBatch, Manifest, Operation, Project, State};
use dockon_persistence::Store;
use dockon_provider_native::Instance;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{
    io::{AsyncRead, BufReader},
    sync::{Mutex, RwLock},
};

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
        // Bound encoded IPC reply below the 1 MiB frame limit, including Unicode.
        while self.entries.len() > 100 {
            self.entries.pop_front();
        }
    }
    pub fn after(&self, seq: u64) -> Vec<Event> {
        self.entries
            .iter()
            .filter(|e| seq > self.seq || e.seq > seq)
            .cloned()
            .collect()
    }
    pub fn epoch(&self) -> &str {
        &self.epoch
    }
    pub fn batch(&self, after: u64) -> EventBatch {
        EventBatch {
            epoch: self.epoch.clone(),
            events: self.after(after),
            truncated: self
                .entries
                .front()
                .is_some_and(|e| after != 0 && after < e.seq.saturating_sub(1)),
        }
    }
}
struct Entry {
    project: RwLock<Project>,
    instance: Mutex<Option<Instance>>,
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
            }),
            instance: Mutex::new(None),
            lifecycle: Mutex::new(()),
        }
    }
}
pub struct Orchestrator {
    entries: RwLock<BTreeMap<String, Arc<Entry>>>,
    store: Store,
    pub events: EventBus,
    closing: AtomicBool,
}
impl Orchestrator {
    pub async fn new(store: Store) -> Result<Self> {
        let mut entries = BTreeMap::new();
        for manifest in store.list().await? {
            // Recovery is deliberately non-autostart. OS ownership cleanup is
            // reconciled separately from the last persisted desired state.
            store.desired(&manifest.id, false).await?;
            entries.insert(manifest.id.clone(), Arc::new(Entry::new(manifest)));
        }
        Ok(Self {
            entries: RwLock::new(entries),
            store,
            events: Arc::default(),
            closing: AtomicBool::new(false),
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
        let mut result = Vec::new();
        for entry in entries {
            if let Ok(_lifecycle) = entry.lifecycle.try_lock() {
                let mut owned = entry.instance.lock().await;
                if let Some(instance) = owned.as_mut() {
                    if let Some(status) = instance.child.try_wait()? {
                        let mut project = entry.project.write().await;
                        project.state = State::Failed;
                        project.pid = None;
                        project.port = None;
                        project.error = Some(format!("Process exited: {status}"));
                        *owned = None;
                        self.events.lock().await.push(
                            &project.manifest.id,
                            "state",
                            "FAILED: process exited",
                        );
                    }
                }
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
            let error = result.err().map(|e| format!("{e:#}"));
            if let Some(message) = &error {
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
    pub async fn operation(&self, id: &str) -> Result<Operation> {
        self.store.operation(id).await
    }
    async fn start_entry(&self, entry: &Entry) -> Result<()> {
        let mut owned = entry.instance.lock().await;
        if let Some(instance) = owned.as_mut() {
            if instance.child.try_wait()?.is_none() {
                return Ok(());
            }
            *owned = None;
        }
        let manifest = {
            let mut project = entry.project.write().await;
            project.state = State::Starting;
            project.pid = None;
            project.port = None;
            project.error = None;
            project.manifest.clone()
        };
        self.events
            .lock()
            .await
            .push(&manifest.id, "state", "STARTING");
        self.store.desired(&manifest.id, true).await?;
        let (instance, port) = start_instance(&manifest, self.events.clone()).await?;
        let mut project = entry.project.write().await;
        project.pid = instance.child.id();
        project.port = Some(port);
        project.state = State::Running;
        *owned = Some(instance);
        self.events
            .lock()
            .await
            .push(&manifest.id, "state", "RUNNING");
        Ok(())
    }
    async fn stop_entry(&self, entry: &Entry) -> Result<()> {
        let mut owned = entry.instance.lock().await;
        let id = entry.project.read().await.manifest.id.clone();
        self.store.desired(&id, false).await?;
        if let Some(instance) = owned.as_mut() {
            entry.project.write().await.state = State::Stopping;
            self.events.lock().await.push(&id, "state", "STOPPING");
            instance.stop(&id).await?;
            *owned = None;
        }
        let mut project = entry.project.write().await;
        project.state = State::Stopped;
        project.pid = None;
        project.port = None;
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
        for entry in entries {
            let _guard = entry.lifecycle.lock().await;
            self.stop_entry(&entry).await?;
        }
        Ok(())
    }
}
async fn start_instance(manifest: &Manifest, events: EventBus) -> Result<(Instance, u16)> {
    manifest.validate()?;
    let reserved =
        std::net::TcpListener::bind(("127.0.0.1", manifest.port)).context("Port unavailable")?;
    let port = reserved.local_addr()?.port();
    drop(reserved);
    let mut instance = Instance::spawn(manifest, port)?;
    if let Some(stdout) = instance.child.stdout.take() {
        capture(stdout, manifest.id.clone(), "stdout", events.clone());
    }
    if let Some(stderr) = instance.child.stderr.take() {
        capture(stderr, manifest.id.clone(), "stderr", events);
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_millis(300))
        .build()?;
    for _ in 0..60 {
        if let Some(status) = instance.child.try_wait()? {
            bail!("Process exited before readiness: {status}");
        }
        if let Ok(response) = client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
        {
            if response.status().is_success() {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if instance.child.try_wait()?.is_none() {
                    return Ok((instance, port));
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    instance.stop(&manifest.id).await?;
    bail!("HTTP readiness timed out; child stopped")
}
fn capture(
    stream: impl AsyncRead + Unpin + Send + 'static,
    id: String,
    kind: &'static str,
    events: EventBus,
) {
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut reader = BufReader::new(stream);
        let mut buffer = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            match reader.read(&mut byte).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if byte[0] == b'\n' || buffer.len() == 2048 {
                        events
                            .lock()
                            .await
                            .push(&id, kind, &String::from_utf8_lossy(&buffer));
                        buffer.clear();
                    }
                    if byte[0] != b'\n' {
                        buffer.push(byte[0]);
                    }
                }
            }
        }
        if !buffer.is_empty() {
            events
                .lock()
                .await
                .push(&id, kind, &String::from_utf8_lossy(&buffer));
        }
    });
}
