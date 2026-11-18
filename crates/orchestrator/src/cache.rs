use super::*;
use dockon_runtime_manager::catalog::{platform, MEMCACHED_WINDOWS, REDIS_WINDOWS};

fn check_service(service: &str) -> Result<()> {
    if !matches!(service, "redis" | "memcached") {
        bail!("Unknown cache service");
    }
    Ok(())
}

impl Orchestrator {
    pub fn cache_catalog(service: &str, container: bool) -> Result<serde_json::Value> {
        check_service(service)?;
        if container || !cfg!(all(windows, target_arch = "x86_64")) {
            bail!("Local cache packages require a native Windows x64 project");
        }
        let versions: Vec<&str> = if service == "redis" {
            REDIS_WINDOWS.iter().map(|(version, _)| *version).collect()
        } else {
            vec![MEMCACHED_WINDOWS]
        };
        Ok(serde_json::json!(versions
            .iter()
            .map(|version| {
                serde_json::json!({
                    "version": version,
                    "installation_id": format!("managed:{service}:{version}:{}", platform())
                })
            })
            .collect::<Vec<_>>()))
    }

    pub async fn submit_cache(
        self: &Arc<Self>,
        id: String,
        service: String,
        version: Option<String>,
    ) -> Result<Operation> {
        self.entry(&id).await?;
        check_service(&service)?;
        let operation = uuid::Uuid::new_v4().to_string();
        self.store.begin(&operation, &id, "configure_cache").await?;
        let reply = self.store.operation(&operation).await?;
        let state = self.clone();
        tokio::spawn(async move {
            let error = state
                .configure_cache(&id, &service, version.as_deref())
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

    async fn configure_cache(&self, id: &str, service: &str, version: Option<&str>) -> Result<()> {
        let entry = self.entry(id).await?;
        let _guard = entry.lifecycle.lock().await;
        let project = entry.project.read().await.clone();
        if project.state != State::Stopped || self.closing.load(Ordering::Acquire) {
            bail!("Stop the project before changing its cache services");
        }
        let mut manifest = project.manifest;
        let previous = manifest
            .services
            .iter()
            .position(|entry| entry.id == service);
        let next = if let Some(version) = version {
            if !cfg!(all(windows, target_arch = "x86_64")) || manifest.provider != Provider::Native
            {
                bail!("Local cache packages require a native Windows x64 project");
            }
            let mut spec = self.native_cache(id, service, version).await?;
            if let Some(index) = previous {
                spec.depends_on
                    .clone_from(&manifest.services[index].depends_on);
                spec.port = manifest.services[index].port;
                spec.startup_timeout_seconds = manifest.services[index].startup_timeout_seconds;
            }
            Some(spec)
        } else {
            None
        };
        match (previous, next) {
            (Some(index), Some(spec)) => manifest.services[index] = spec,
            (None, Some(spec)) => manifest.services.push(spec),
            (Some(index), None) => {
                manifest.services.remove(index);
            }
            (None, None) => return Ok(()),
        }
        manifest.validate()?;
        self.store.update_manifest(&manifest).await?;
        entry.project.write().await.manifest = manifest;
        Ok(())
    }

    async fn native_cache(
        &self,
        project: &str,
        service: &str,
        version: &str,
    ) -> Result<dockon_core::ServiceSpec> {
        let events = self.events.clone();
        let project = project.to_owned();
        let installation = self
            .runtimes
            .install(
                service,
                version,
                Arc::new(move |message| {
                    let events = events.clone();
                    let project = project.clone();
                    tokio::spawn(async move {
                        events.lock().await.push(&project, "runtime", &message);
                    });
                }),
            )
            .await?;
        let args = if service == "redis" {
            serde_json::json!(["--bind", "127.0.0.1", "--port", {"template": "${DOCKON_PORT}"},
                "--dir", {"template": "${DOCKON_DATA}"}, "--protected-mode", "yes", "--appendonly", "yes"])
        } else {
            serde_json::json!(["-l", "127.0.0.1", "-p", {"template": "${DOCKON_PORT}"}, "-U", "0"])
        };
        Ok(serde_json::from_value(serde_json::json!({
            "id": service, "executable": installation.executable, "args": args,
            "runtime": {"installation_id": installation.id}, "health": {"type": "tcp"}
        }))?)
    }
}
