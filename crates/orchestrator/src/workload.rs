use crate::ports::{PortAllocator, PortLease};
use crate::EventBus;
use anyhow::{bail, Context, Result};
use dockon_core::{HealthCheck, Manifest, Provider};
use dockon_provider_container::{ContainerInstance, ContainerProvider};
use dockon_provider_native::Instance;
use std::{collections::BTreeMap, path::Path};
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};

pub(crate) enum OwnedInstance {
    Native {
        process: Instance,
        _lease: PortLease,
    },
    Container {
        process: ContainerInstance,
        _lease: PortLease,
    },
}
impl OwnedInstance {
    pub fn pid(&self) -> Option<u32> {
        match self {
            Self::Native { process, .. } => process.child.id(),
            Self::Container { .. } => None,
        }
    }
    pub fn container_id(&self) -> Option<String> {
        match self {
            Self::Native { .. } => None,
            Self::Container { process, .. } => Some(process.id.clone()),
        }
    }
    pub async fn running(&mut self) -> Result<bool> {
        match self {
            Self::Native { process, .. } => Ok(process.child.try_wait()?.is_none()),
            Self::Container { process, .. } => process.running().await,
        }
    }
    pub async fn stop(&mut self, owner: &str) -> Result<()> {
        match self {
            Self::Native { process, .. } => process.stop(owner).await,
            Self::Container { process, .. } => process.stop(owner).await,
        }
    }
}

pub(crate) async fn launch(
    manifest: &Manifest,
    events: EventBus,
    providers: (
        &ContainerProvider,
        &PortAllocator,
        &dockon_runtime_manager::RuntimeManager,
    ),
    data_dir: &Path,
    project: &str,
    alias: &str,
    environment: &BTreeMap<String, String>,
) -> Result<(OwnedInstance, u16)> {
    manifest.validate()?;
    let (containers, ports, runtimes) = providers;
    let lease = ports.reserve(manifest.port)?;
    let port = lease.port;
    let mut instance = match manifest.provider {
        Provider::Native => {
            let (prepared, environment) = runtimes.prepare(manifest, data_dir, environment).await?;
            let mut instance = Instance::spawn(&prepared, port, data_dir, &environment)?;
            if let Some(stdout) = instance.child.stdout.take() {
                capture(
                    stdout,
                    project.into(),
                    alias.into(),
                    "stdout",
                    events.clone(),
                );
            }
            if let Some(stderr) = instance.child.stderr.take() {
                capture(
                    stderr,
                    project.into(),
                    alias.into(),
                    "stderr",
                    events.clone(),
                );
            }
            OwnedInstance::Native {
                process: instance,
                _lease: lease,
            }
        }
        Provider::Container => {
            let spec = manifest
                .container
                .as_ref()
                .context("Container configuration required")?;
            events.lock().await.push(
                project,
                "provider",
                &format!("Preparing pinned image and isolated resources for {alias}"),
            );
            OwnedInstance::Container {
                process: containers
                    .start(project, &manifest.id, alias, spec, port, environment)
                    .await?,
                _lease: lease,
            }
        }
    };
    let ready = wait_ready(manifest, &mut instance, port).await;
    if let Err(error) = ready {
        if let OwnedInstance::Container {
            process: container, ..
        } = &instance
        {
            if let Ok(logs) = container.logs().await {
                events.lock().await.push(project, "stderr", &logs);
            }
        }
        if let Err(cleanup) = instance.stop(&manifest.id).await {
            bail!("{error:#}; workload cleanup failed: {cleanup:#}");
        }
        return Err(error);
    }
    Ok((instance, port))
}
async fn wait_ready(manifest: &Manifest, instance: &mut OwnedInstance, port: u16) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_millis(300))
        .build()?;
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(manifest.startup_timeout_seconds);
    while tokio::time::Instant::now() < deadline {
        if !instance.running().await? {
            bail!("Workload exited before readiness");
        }
        let ready = match &manifest.health {
            HealthCheck::Http { path } => client
                .get(format!("http://127.0.0.1:{port}{path}"))
                .send()
                .await
                .is_ok_and(|r| r.status().is_success()),
            HealthCheck::Tcp => tokio::time::timeout(
                std::time::Duration::from_millis(300),
                tokio::net::TcpStream::connect(("127.0.0.1", port)),
            )
            .await
            .is_ok_and(|r| r.is_ok()),
        };
        if ready {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            if instance.running().await? {
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    bail!("Readiness timed out")
}
fn capture(
    stream: impl AsyncRead + Unpin + Send + 'static,
    id: String,
    alias: String,
    kind: &'static str,
    events: EventBus,
) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stream);
        let mut buffer = vec![];
        loop {
            let mut byte = [0u8; 1];
            match reader.read(&mut byte).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if byte[0] == b'\n' || buffer.len() == 2048 {
                        let text = String::from_utf8_lossy(&buffer);
                        events.lock().await.push(
                            &id,
                            kind,
                            &if alias == "app" {
                                text.into_owned()
                            } else {
                                format!("[{alias}] {text}")
                            },
                        );
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
