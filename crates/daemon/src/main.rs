mod advisories;
mod diagnostics;
mod guardian;
mod project_tools;
mod workspace;

use anyhow::Result;
use dockon_core::{Command, Manifest, Request, Response};
use dockon_ipc::{read_json, write_json, Connection};
use dockon_orchestrator::{EventBus, Orchestrator};
use fs2::FileExt;
use std::sync::Arc;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::Notify,
};

struct DaemonState {
    projects: Arc<Orchestrator>,
    runtimes: Arc<dockon_runtime_manager::RuntimeManager>,
    store: dockon_persistence::Store,
    events: EventBus,
    shutdown: Arc<Notify>,
    closing: std::sync::atomic::AtomicBool,
    router: Arc<dockon_router::Router>,
    workspace_routes: tokio::sync::Mutex<()>,
}
impl DaemonState {
    async fn router_settings(&self) -> Result<dockon_core::RouterSettings> {
        if let Some(settings) = self.router.status().await?.settings {
            return Ok(settings);
        }
        let caddy = self
            .runtimes
            .installed()
            .await?
            .into_iter()
            .find(|runtime| runtime.runtime == "caddy")
            .ok_or_else(|| anyhow::anyhow!("Install Caddy to enable local addresses"))?;
        Ok(dockon_core::RouterSettings {
            runtime_installation: caddy.id,
            http_port: local_web_port(80, 8080)?,
            https_port: local_web_port(443, 8443)?,
            domain_suffix: dockon_core::default_domain_suffix(),
            default_https: false,
            routes: vec![],
        })
    }

    async fn router_preferences(
        &self,
        domain_suffix: String,
        default_https: bool,
    ) -> Result<dockon_router::Status> {
        let _guard = self.workspace_routes.lock().await;
        let suffix = domain_suffix
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        dockon_router::validate_domain(&suffix)?;
        if suffix.contains('.')
            || suffix == "localhost"
            || suffix.bytes().all(|b| b.is_ascii_digit())
        {
            anyhow::bail!("Enter a single local domain suffix, for example .test or .local");
        }
        let mut settings = self.router_settings().await?;
        if suffix != settings.domain_suffix {
            let old_suffix = format!(".{}", settings.domain_suffix);
            for route in &mut settings.routes {
                if let Some(host) = route.domain.strip_suffix(&old_suffix) {
                    route.domain = format!("{host}.{suffix}");
                }
            }
            settings.domain_suffix = suffix;
        }
        settings.default_https = default_https;
        let projects = self.projects.list().await?;
        self.router.save_preferences(settings, &projects).await?;
        self.router.status().await
    }

    async fn route_workspace(
        &self,
        id: &str,
        https: Option<bool>,
    ) -> Result<dockon_router::Status> {
        let _guard = self.workspace_routes.lock().await;
        if self.closing.load(std::sync::atomic::Ordering::Acquire) {
            anyhow::bail!("Daemon is shutting down");
        }
        let projects = self.projects.list().await?;
        let project = projects
            .iter()
            .find(|project| project.manifest.id == id)
            .ok_or_else(|| anyhow::anyhow!("Project not found"))?;
        let status = self.router.status().await?;
        if https.is_none()
            && !project.has_mailpit()
            && matches!(project.manifest.health, dockon_core::HealthCheck::Tcp)
        {
            return Ok(status);
        }
        let mut settings = self.router_settings().await?;
        let domain = self.router.project_domain(&settings, project, &projects)?;
        if status.running
            && https.is_none()
            && settings
                .routes
                .iter()
                .any(|route| route.project_id == id && route.domain == domain)
        {
            return Ok(status);
        }
        if https.is_none() && !status.running {
            if settings.http_port == 80 && !dockon_router::loopback_port_available(80) {
                settings.http_port = local_web_port(80, 8080)?;
            }
            if settings.https_port == 443 && !dockon_router::loopback_port_available(443) {
                settings.https_port = local_web_port(443, 8443)?;
            }
        }
        if let Some(route) = settings
            .routes
            .iter_mut()
            .find(|route| route.project_id == id)
        {
            route.domain = domain;
            if let Some(https) = https {
                route.https = https;
            }
        } else {
            settings.routes.push(dockon_core::ProjectRoute {
                project_id: id.into(),
                domain,
                https: https.unwrap_or(settings.default_https),
            });
        }
        let plan = self.router.plan(settings, &projects).await?;
        self.router.apply(&plan.token, &projects).await?;
        self.router.status().await
    }

    async fn submit_template(
        self: &Arc<Self>,
        id: String,
        name: String,
        root: std::path::PathBuf,
        installation: String,
        cgi: Option<String>,
        template: String,
        tools: dockon_core::ProjectTools,
        services: Vec<dockon_core::ServiceSpec>,
    ) -> Result<dockon_core::Operation> {
        let operation = uuid::Uuid::new_v4().to_string();
        self.store.begin(&operation, &id, "project.create").await?;
        let reply = self.store.operation(&operation).await?;
        let state = self.clone();
        tokio::spawn(async move {
            let result: Result<()> = async {
                let mut manifest = dockon_runtime_manager::templates::create(
                    &state.runtimes,
                    &id,
                    &name,
                    &root,
                    &installation,
                    cgi.as_deref(),
                    &template,
                    tools,
                )
                .await?;
                manifest.services.extend(services);
                state.projects.register(manifest).await?;
                Ok(())
            }
            .await;
            let error = result.err().map(|error| format!("{error:#}"));
            if let Err(error) = state.store.finish(&operation, error.as_deref()).await {
                tracing::error!(%error, "Unable to record template creation outcome");
            }
        });
        Ok(reply)
    }

    async fn system_apply(self: &Arc<Self>, token: String) -> Result<dockon_core::Operation> {
        let id = uuid::Uuid::new_v4().to_string();
        self.store.begin(&id, "system", "system.apply").await?;
        let reply = self.store.operation(&id).await?;
        let state = self.clone();
        tokio::spawn(async move {
            let error = state
                .router
                .permission_apply(&token)
                .await
                .err()
                .map(|e| format!("{e:#}"));
            if let Err(error) = state.store.finish(&id, error.as_deref()).await {
                tracing::error!(%error,"Unable to record system permission outcome");
            }
        });
        Ok(reply)
    }
    async fn install(
        self: &Arc<Self>,
        runtime: String,
        version: String,
    ) -> Result<dockon_core::Operation> {
        let id = uuid::Uuid::new_v4().to_string();
        let owner = format!("runtime:{runtime}");
        self.store.begin(&id, &owner, "runtime.install").await?;
        let response = self.store.operation(&id).await?;
        let state = self.clone();
        tokio::spawn(async move {
            let (send, mut receive) = tokio::sync::mpsc::unbounded_channel::<String>();
            let events = state.events.clone();
            let event_owner = owner.clone();
            let logger = tokio::spawn(async move {
                while let Some(message) = receive.recv().await {
                    events.lock().await.push(&event_owner, "runtime", &message);
                }
            });
            let progress: dockon_runtime_manager::Progress = Arc::new(move |message| {
                let _ = send.send(message);
            });
            let result = state.runtimes.install(&runtime, &version, progress).await;
            let _ = logger.await;
            let error = result.err().map(|e| format!("{e:#}"));
            if let Some(error) = &error {
                state.events.lock().await.push(&owner, "error", error);
            }
            if let Err(error) = state.store.finish(&id, error.as_deref()).await {
                tracing::error!(%error,"Unable to record runtime operation outcome");
            }
        });
        Ok(response)
    }
}

fn local_web_port(standard: u16, alternate: u16) -> Result<u16> {
    cfg!(windows)
        .then_some(standard)
        .into_iter()
        .chain(alternate..=alternate + 9)
        .find(|port| dockon_router::loopback_port_available(*port))
        .ok_or_else(|| anyhow::anyhow!("No free local router port near {alternate}"))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter("info")
        .init();
    let dir = dockon_ipc::prepare_private_dir()?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("daemon.lock"))?;
    lock.try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("Dockon daemon is already running"))?;
    let store = dockon_persistence::Store::open(&dir.join("dockon.sqlite")).await?;
    let runtimes = Arc::new(dockon_runtime_manager::RuntimeManager::new(
        dir.clone(),
        store.clone(),
    )?);
    #[cfg(windows)]
    if let Some(directory) = std::env::current_exe()?.parent() {
        if let Err(error) = runtimes.register_bundled_web(directory).await {
            tracing::warn!(%error, "Could not register bundled web runtimes");
        }
    }
    let orchestrator = Orchestrator::new(store.clone(), dir.clone()).await?;
    let events = orchestrator.events.clone();
    let shutdown = Arc::new(Notify::new());
    let state = Arc::new(DaemonState {
        router: Arc::new(dockon_router::Router::new(dir.clone(), runtimes.clone())?),
        projects: Arc::new(orchestrator),
        runtimes,
        store,
        events,
        shutdown: shutdown.clone(),
        closing: false.into(),
        workspace_routes: tokio::sync::Mutex::new(()),
    });
    let router_state = state.clone();
    let router_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if let Ok(projects) = router_state.projects.list().await {
                if let Err(error) = router_state.router.refresh(&projects).await {
                    tracing::warn!(%error,"Router refresh failed");
                }
            }
        }
    });
    let token = uuid::Uuid::new_v4().to_string();
    #[cfg(windows)]
    let endpoint = format!(r"\\.\pipe\dockon-{}", uuid::Uuid::new_v4());
    #[cfg(unix)]
    let endpoint = dir
        .join("daemon.sock")
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Non-UTF8 socket path"))?
        .to_owned();
    #[cfg(unix)]
    let listener = {
        let _ = std::fs::remove_file(&endpoint);
        tokio::net::UnixListener::bind(&endpoint)?
    };
    #[cfg(windows)]
    let mut listener = tokio::net::windows::named_pipe::ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&endpoint)?;
    let connection = Connection {
        endpoint: endpoint.clone(),
        token: token.clone(),
    };
    std::fs::write(dir.join("connection.tmp"), serde_json::to_vec(&connection)?)?;
    std::fs::rename(dir.join("connection.tmp"), dir.join("connection.json"))?;
    tracing::info!("Dockon daemon ready");
    loop {
        #[cfg(windows)]
        let accepted = listener.connect();
        #[cfg(unix)]
        let accepted = listener.accept();
        tokio::select! {
            _ = shutdown.notified() => break,
            _ = tokio::signal::ctrl_c() => break,
            incoming = accepted => {
                #[cfg(windows)]
                let stream = {
                    incoming?;
                    let next = tokio::net::windows::named_pipe::ServerOptions::new().reject_remote_clients(true).create(&endpoint)?;
                    std::mem::replace(&mut listener, next)
                };
                #[cfg(unix)]
                let (stream, _) = incoming?;
                let state = state.clone(); let token = token.clone();
                tokio::spawn(async move { if let Err(error) = serve(stream, token, state).await { tracing::warn!(%error, "IPC connection failed"); } });
            }
        }
    }
    router_task.abort();
    state
        .closing
        .store(true, std::sync::atomic::Ordering::Release);
    {
        let _guard = state.workspace_routes.lock().await;
        state.router.stop().await?;
    }
    state.projects.shutdown().await?;
    std::fs::remove_file(dir.join("connection.json"))?;
    #[cfg(unix)]
    std::fs::remove_file(endpoint)?;
    tracing::info!("Dockon daemon stopped");
    Ok(())
}

async fn serve(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    token: String,
    state: Arc<DaemonState>,
) -> Result<()> {
    let request: Request =
        tokio::time::timeout(std::time::Duration::from_secs(5), read_json(&mut stream)).await??;
    if request.version != 1 || request.token != token {
        write_json(
            &mut stream,
            &Response::failure("Unauthorized or unsupported protocol"),
        )
        .await?;
        return Ok(());
    }
    if let Command::Subscribe { after } = request.command {
        let mut cursor = after;
        loop {
            let batch = state.events.lock().await.batch(cursor);
            if let Some(last) = batch.events.last() {
                cursor = last.seq;
            }
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                write_json(&mut stream, &Response::success(batch)),
            )
            .await??;
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }
    let response: Result<Response> = async {
        Ok(match request.command {
            Command::SystemPlan {action} => Response::success(state.router.permission_plan(action, &state.projects.list().await?).await?),
            Command::SystemApply {token} => Response::success(state.system_apply(token).await?),
            Command::RouterStatus => {
                let status = state.router.status().await?;
                let pending = match &status.settings {
                    Some(settings) => state.router.hosts_pending(settings, &state.projects.list().await?).unwrap_or(false),
                    None => false,
                };
                let mut value = serde_json::to_value(status)?;
                value["hosts_pending"] = pending.into();
                Response::success(value)
            },
            Command::RouterPreferences { domain_suffix, default_https } =>
                Response::success(state.router_preferences(domain_suffix, default_https).await?),
            Command::RouterPlan {settings} => Response::success(state.router.plan(settings,&state.projects.list().await?).await?),
            Command::RouterApply { token } => {
                let _guard = state.workspace_routes.lock().await;
                state.router.apply(&token, &state.projects.list().await?).await?;
                Response::success(state.router.status().await?)
            },
            Command::RouterStop => {
                let _guard = state.workspace_routes.lock().await;
                state.router.stop().await?;
                Response::success(state.router.status().await?)
            },
            Command::Health => Response::success(serde_json::json!({"name":"dockon", "version":env!("CARGO_PKG_VERSION"), "native":"http_tcp", "container":false, "ai":"not_implemented"})),
            Command::Diagnostics => Response::success(diagnostics::report(&state).await?),
            Command::WorkspaceFolders => Response::success(workspace::folders()?),
            Command::ProjectPhpVersion { id, runtime_installation } => Response::success(state.projects.set_php_version(&id, &runtime_installation).await?),
            Command::ProjectRuntimeVersion { id, runtime_installation } => Response::success(state.projects.set_runtime_version(&id, &runtime_installation).await?),
            Command::WorkspaceGitClone { slug, url } => Response::success(state.git_clone(slug,url).await?),
            Command::ProjectPresetCapture { id, name } => Response::success(state.capture_preset(&id,name).await?),
            Command::ProjectPresets => Response::success(project_tools::list_presets()?),
            Command::ProjectPresetSave { preset } => Response::success(project_tools::save_preset(preset)?),
            Command::ProjectPresetRemove { id } => Response::success(project_tools::remove_preset(id)?),
            Command::ProjectCheck { id } => Response::success(state.check_project(&id).await?),
            Command::ProjectTaskRun { id, task, device } => Response::success(state.submit_task(id,task,device).await?),
            Command::ProjectTaskStop { id, task } => Response::success(state.projects.stop_project_task(&id, task).await?),
            Command::ProjectTasks { id } => Response::success(state.projects.project_tasks(&id).await?),
            Command::ProjectTasksSet { id,tasks } => Response::success(state.projects.set_project_tasks(&id,tasks).await?),
            Command::FlutterEmulators { id } => Response::success(state.projects.flutter_emulators(&id).await?),
            Command::FlutterTargets { id } => Response::success(state.projects.flutter_targets(&id).await?),
            Command::DatabaseSnapshot { id,engine } => Response::success(state.projects.submit_snapshot(id,engine).await?),
            Command::DatabaseSnapshots { id,engine } => Response::success(state.projects.snapshots(&id,&engine).await?),
            Command::DatabaseSnapshotCopy { id,engine,backup,target } => Response::success(state.projects.submit_snapshot_copy(id,engine,backup,target).await?),
            Command::ProjectToolsSet { id, tools } => Response::success(state.projects.set_tools(&id, tools).await?),
            Command::ProjectToolEnvironment { id } => Response::success(state.projects.tool_environment(&id).await?),
            Command::ProjectRuntimePaths { id } => Response::success(state.projects.runtime_paths(&id).await?),
            Command::CacheCatalog { service, container } => Response::success(Orchestrator::cache_catalog(&service, container)?),
            Command::ProjectCacheSet { id, service, version } => Response::success(state.projects.submit_cache(id, service, version).await?),
            Command::WorkspaceCreate { slug, name, runtime_installation, php_cgi_installation, database, mail, template, tools } => {
                let mail = mail.then(Orchestrator::mail_service).transpose()?;
                let database = database.as_deref().map(Orchestrator::database_service).transpose()?;
                let root = workspace::project_path(&slug)?;
                if state.projects.list().await?.iter().any(|p| p.manifest.id == slug) {
                    anyhow::bail!("A project with this folder name already exists");
                }
                if let Some(template) = template {
                    return Ok(Response::success(state.submit_template(slug, name, root, runtime_installation, php_cgi_installation, template, tools, database.into_iter().chain(mail).collect()).await?));
                }
                let mut manifest = dockon_runtime_manager::projects::create_with_php(&state.runtimes, &slug, &name, &root, &runtime_installation, php_cgi_installation.as_deref()).await?;
                manifest.tools = tools;
                state.runtimes.tool_environment(&manifest).await?;
                manifest.services.extend(database);
                manifest.services.extend(mail);
                Response::success(state.projects.register(manifest).await?)
            },
            Command::WorkspaceEnsureRoute { id } => Response::success(state.route_workspace(&id, None).await?),
            Command::WorkspaceRoute { id, https } => Response::success(state.route_workspace(&id, Some(https)).await?),
            Command::StackSet { .. } | Command::LocalServices | Command::LocalServicesSet { .. }
            | Command::SharedDatabaseStatus | Command::SharedDatabaseSet { .. }
            | Command::SharedWebStatus | Command::SharedWebSet { .. } | Command::SharedWebReload =>
                Response::failure("Shared services have been replaced by project services"),
            Command::GuardianPreview => Response::success(serde_json::json!({
                "provider_base_url": guardian::BASE_URL,
                "context": diagnostics::report(&state).await?
            })),
            Command::GuardianModels { api_key } => Response::success(guardian::models(&api_key).await?),
            Command::GuardianAnalyze { api_key, model, context } => {
                let approved = diagnostics::report(&state).await?;
                if context != approved { anyhow::bail!("Diagnostics changed; preview the current context before sending"); }
                Response::success(guardian::analyze(&api_key, &model, approved).await?)
            },
            Command::AdvisoryLookup { ecosystem, package, version } => Response::success(advisories::lookup(&ecosystem, &package, &version).await?),
            Command::RuntimeUpdates { runtime } => Response::success(state.runtimes.updates(&runtime).await?),
            Command::Capabilities => Response::success(serde_json::json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"native":"http_tcp","container":false,"ai":"not_implemented"})),
            Command::RuntimeCatalog { runtime } => Response::success(state.runtimes.catalog(&runtime).await?),
            Command::RuntimeInstalled => Response::success(state.runtimes.installed().await?),
            Command::RuntimeInstall { runtime,version } => Response::success(state.install(runtime,version).await?),
            Command::RuntimeDiscover => Response::success(state.runtimes.discover()),
            Command::RuntimeUseSystem { runtime,executable } => Response::success(state.runtimes.use_system(&runtime,&executable).await?),
            Command::ProjectDetect {root} => Response::success(dockon_runtime_manager::projects::detect(&root)?),
            Command::ProjectCreate {id,name,root,runtime_installation,php_cgi_installation,database,mail,template,tools} => {
                let mail = mail.then(Orchestrator::mail_service).transpose()?;
                let database = database.as_deref().map(Orchestrator::database_service).transpose()?;
                if state.projects.list().await?.iter().any(|p|p.manifest.id==id){anyhow::bail!("Project id is already registered");}
                if let Some(template) = template {
                    return Ok(Response::success(state.submit_template(id, name, root, runtime_installation, php_cgi_installation, template, tools, database.into_iter().chain(mail).collect()).await?));
                }
                let mut manifest = dockon_runtime_manager::projects::create_with_php(&state.runtimes, &id, &name, &root, &runtime_installation, php_cgi_installation.as_deref()).await?;
                manifest.tools = tools;
                state.runtimes.tool_environment(&manifest).await?;
                manifest.services.extend(database);
                manifest.services.extend(mail);
                Response::success(state.projects.register(manifest).await?)
            },
            Command::ProjectImport {id,name,root,runtime_installation,profile,php_cgi_installation,database,mail,tools} => {
                let mail = mail.then(Orchestrator::mail_service).transpose()?;
                let database = database.as_deref().map(Orchestrator::database_service).transpose()?;
                let mut manifest=dockon_runtime_manager::projects::import(&state.runtimes,&id,&name,&root,&runtime_installation,&profile).await?;
                dockon_runtime_manager::projects::attach_php_cgi(&state.runtimes, &mut manifest, &profile, php_cgi_installation.as_deref()).await?;
                manifest.tools = tools;
                state.runtimes.tool_environment(&manifest).await?;
                manifest.services.extend(database);
                manifest.services.extend(mail);
                Response::success(state.projects.register(manifest).await?)
            },
            Command::ProjectSwitchStaticServer { id, runtime_installation } => Response::success(
                state.projects.submit_static_switch(id, runtime_installation).await?
            ),
            Command::List => Response::success(state.projects.list().await?),
            Command::ProjectAddRedis { id } => Response::success(state.projects.add_redis(&id).await?),
            Command::ProjectAddMailpit { id } => Response::success(state.projects.add_mailpit(&id).await?),
            Command::ProjectSetMail { id, enabled } => Response::success(state.projects.set_mail(&id, enabled).await?),
            Command::ProjectAddPostgres { id } => Response::success(state.projects.add_postgres(&id).await?),
            Command::ProjectAddDatabase { id, engine } => Response::success(state.projects.add_database(&id, &engine).await?),
            Command::ProjectMigratePostgres { id, dockon_dump, postgres_dump } => Response::success(state.projects.migrate_postgres(&id, &dockon_dump, &postgres_dump).await?),
            Command::PostgresConnection { id } => Response::success(state.projects.postgres_connection(&id).await?),
            Command::DatabaseConnection { id, engine } => Response::success(state.projects.database_connection(&id, &engine).await?),
            Command::PostgresBackup { id } => Response::success(state.projects.backup_postgres(&id).await?),
            Command::PostgresBackups { id } => Response::success(state.projects.postgres_backups(&id).await?),
            Command::PostgresRestore { id, backup } => Response::success(state.projects.submit_postgres_restore(id, backup).await?),
            Command::Register { manifest_path } => Response::success(state.projects.register(Manifest::read(&manifest_path)?).await?),
            Command::Start { id } => {
                let operation = state.projects.submit(id.clone(), true).await?;
                let operation_id = operation.id.clone();
                let state = state.clone();
                tokio::spawn(async move {
                    for _ in 0..2400 {
                        if state.closing.load(std::sync::atomic::Ordering::Acquire) {
                            break;
                        }
                        match state.store.operation(&operation_id).await {
                            Ok(operation) if operation.status == "completed" => {
                                if let Err(error) = state.route_workspace(&id, None).await {
                                    state.events.lock().await.push(&id, "error", &format!("Local URL: {error:#}"));
                                }
                                break;
                            }
                            Ok(operation) if operation.status == "running" => {
                                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                            }
                            _ => break,
                        }
                    }
                });
                Response::success(operation)
            },
            Command::Stop { id } => Response::success(state.projects.submit(id, false).await?),
            Command::Operation { id } => Response::success(state.store.operation(&id).await?),
            Command::Events { after } => {
                let feed = state.events.lock().await;
                Response::success(feed.batch(after))
            },
            Command::Subscribe { .. } => unreachable!(),
            Command::Shutdown => {
                state.shutdown.notify_one();
                Response::success(serde_json::json!({"shutting_down":true}))
            }
        })
    }.await;
    write_json(
        &mut stream,
        &response.unwrap_or_else(|error| Response::failure(format!("{error:#}"))),
    )
    .await?;
    Ok(())
}
