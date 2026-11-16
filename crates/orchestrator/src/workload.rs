use crate::local_db::{Database, Kind};
use crate::ports::{PortAllocator, PortLease};
use crate::EventBus;
use anyhow::{bail, Context, Result};
use dockon_core::{HealthCheck, Manifest, Provider};
use dockon_provider_native::Instance;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};

pub(crate) struct OwnedInstance {
    process: Instance,
    _lease: PortLease,
    smtp: Option<PortLease>,
    database: Option<Database>,
}
impl OwnedInstance {
    pub fn database(&self) -> Option<&Database> {
        self.database.as_ref()
    }
    pub fn smtp_port(&self) -> Option<u16> {
        self.smtp.as_ref().map(|lease| lease.port)
    }
    pub fn pid(&self) -> Option<u32> {
        self.process.child.id()
    }
    pub async fn running(&mut self) -> Result<bool> {
        Ok(self.process.child.try_wait()?.is_none())
    }
    pub async fn stop(&mut self, owner: &str) -> Result<()> {
        if let Some(database) = &self.database {
            if self.process.child.try_wait()?.is_some() {
                return Ok(());
            }
            if database.stop().await.is_ok()
                && tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    self.process.child.wait(),
                )
                .await
                .is_ok_and(|result| result.is_ok())
            {
                return Ok(());
            }
        }
        self.process.stop(owner).await
    }
}

pub fn bundled_mailpit_path() -> Result<Option<PathBuf>> {
    if !cfg!(windows) {
        return Ok(None);
    }
    let exe = std::env::current_exe()?;
    Ok(exe
        .parent()
        .map(|dir| dir.join("bin/mailpit/mailpit.exe"))
        .filter(|file| file.is_file()))
}

pub(crate) async fn launch(
    manifest: &Manifest,
    events: EventBus,
    providers: (&PortAllocator, &dockon_runtime_manager::RuntimeManager),
    data_dir: &Path,
    project: &str,
    alias: &str,
    environment: &BTreeMap<String, String>,
) -> Result<(OwnedInstance, u16)> {
    manifest.validate()?;
    let (ports, runtimes) = providers;
    let lease = ports.reserve(manifest.port)?;
    let port = lease.port;
    if manifest.provider != Provider::Native {
        bail!("Choose a local workload");
    }
    let (mut prepared, environment) = runtimes
        .prepare(manifest, data_dir, environment, port)
        .await?;
    let database = if let Ok(kind) = Kind::parse(alias) {
        if kind.matches(&prepared.executable) && prepared.args.is_empty() {
            Some(Database::prepare(kind, &mut prepared, data_dir, port).await?)
        } else {
            None
        }
    } else {
        None
    };
    let smtp = if alias == "mailpit"
        && prepared.executable.ends_with("bin/mailpit/mailpit.exe")
        && prepared.args.is_empty()
    {
        prepared.executable =
            bundled_mailpit_path()?.context("Local Mailpit is missing from the Dockon package")?;
        let smtp = ports.reserve(0)?;
        prepared.args.extend(
            [
                format!("--listen=127.0.0.1:{port}"),
                format!("--smtp=127.0.0.1:{}", smtp.port),
                format!("--database={}", data_dir.join("mailpit.db").display()),
                "--disable-version-check".into(),
                "--allowed-hosts=127.0.0.1,localhost,mailpit".into(),
            ]
            .into_iter()
            .map(dockon_core::LaunchArg::from),
        );
        Some(smtp)
    } else {
        None
    };
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
    let mut instance = OwnedInstance {
        process: instance,
        _lease: lease,
        smtp,
        database,
    };
    let ready = async {
        wait_ready(manifest, &mut instance, port).await?;
        if let Some(database) = instance.database() {
            database.initialize().await?;
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(error) = ready {
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
            if instance.running().await?
                && match instance.database() {
                    Some(database) => database.ready().await,
                    None => true,
                }
            {
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    bail!("Readiness timed out")
}
pub(crate) fn capture(
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
