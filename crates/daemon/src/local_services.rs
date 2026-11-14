use anyhow::{bail, Context, Result};
use dockon_core::Manifest;
use dockon_provider_native::Instance;
use serde::Serialize;
use serde_json::json;
use std::{path::PathBuf, time::Duration};
use tokio::sync::Mutex;

pub struct Services {
    data: PathBuf,
    mailpit: PathBuf,
    instance: Mutex<Option<Instance>>,
}

#[derive(Serialize)]
pub struct Status {
    mailpit_available: bool,
    mailpit_running: bool,
}

impl Services {
    pub fn new(data: PathBuf) -> Result<Self> {
        let executable = std::env::current_exe()?;
        let directory = executable
            .parent()
            .context("Application directory unavailable")?;
        Ok(Self {
            data,
            mailpit: directory.join("bin/mailpit/mailpit.exe"),
            instance: Mutex::new(None),
        })
    }

    pub async fn status(&self) -> Result<Status> {
        self.status_for(&mut *self.instance.lock().await)
    }

    fn status_for(&self, instance: &mut Option<Instance>) -> Result<Status> {
        let running = match instance {
            Some(process) => process.child.try_wait()?.is_none(),
            None => false,
        };
        if !running {
            *instance = None;
        }
        Ok(Status {
            mailpit_available: cfg!(windows) && self.mailpit.is_file(),
            mailpit_running: running,
        })
    }

    pub async fn set(&self, running: bool) -> Result<Status> {
        let mut slot = self.instance.lock().await;
        let status = self.status_for(&mut slot)?;
        if running == status.mailpit_running {
            return Ok(status);
        }
        if !running {
            if let Some(process) = slot.as_mut() {
                process.stop("dockon-mailpit").await?;
            }
            *slot = None;
            return self.status_for(&mut slot);
        }
        if !status.mailpit_available {
            bail!("The bundled Mailpit executable is missing");
        }
        for port in [1025, 8025] {
            std::net::TcpListener::bind(("127.0.0.1", port))
                .with_context(|| format!("Mailpit port {port} is already in use"))?;
        }
        let data = self.data.join("mailpit");
        std::fs::create_dir_all(&data)?;
        let manifest: Manifest = serde_json::from_value(json!({
            "schema_version": 1, "id": "dockon-mailpit", "name": "Mailpit",
            "root": data, "executable": self.mailpit,
            "args": ["--listen", "127.0.0.1:8025", "--smtp", "127.0.0.1:1025", "--database", data.join("mailpit.db")],
        }))?;
        let mut process = Instance::spawn(&manifest, 8025, &data, &Default::default())?;
        if let Some(mut stdout) = process.child.stdout.take() {
            tokio::spawn(async move {
                let _ = tokio::io::copy(&mut stdout, &mut tokio::io::sink()).await;
            });
        }
        if let Some(mut stderr) = process.child.stderr.take() {
            tokio::spawn(async move {
                let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
            });
        }
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(1))
            .build()?;
        for _ in 0..50 {
            if let Some(exit) = process.child.try_wait()? {
                bail!("Mailpit exited during startup ({exit})");
            }
            if http
                .get("http://127.0.0.1:8025/api/v1/messages")
                .send()
                .await
                .is_ok_and(|reply| reply.status().is_success())
                && tokio::net::TcpStream::connect(("127.0.0.1", 1025))
                    .await
                    .is_ok()
            {
                *slot = Some(process);
                return self.status_for(&mut slot);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        process.stop("dockon-mailpit").await?;
        bail!("Mailpit did not become ready")
    }
}
