mod diagnostics;

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
    router: Arc<dockon_router::Router>,
}
impl DaemonState {
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
    state.router.stop().await?;
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
            Command::SystemPlan {action} => Response::success(state.router.permission_plan(action).await?),
            Command::SystemApply {token} => Response::success(state.system_apply(token).await?),
            Command::RouterStatus => Response::success(state.router.status().await?),
            Command::RouterPlan {settings} => Response::success(state.router.plan(settings,&state.projects.list().await?).await?),
            Command::RouterApply {token} => {state.router.apply(&token,&state.projects.list().await?).await?;Response::success(state.router.status().await?)},
            Command::RouterStop => {state.router.stop().await?;Response::success(state.router.status().await?)},
            Command::Health => Response::success(serde_json::json!({"name":"dockon", "version":env!("CARGO_PKG_VERSION"), "native":"http_tcp", "container":"digest_pinned_http_tcp", "ai":"not_implemented"})),
            Command::Diagnostics => Response::success(diagnostics::report(&state).await?),
            Command::Capabilities => {
                let (docker,podman)=tokio::join!(dockon_provider_container::capability(dockon_core::ContainerEngine::Docker),dockon_provider_container::capability(dockon_core::ContainerEngine::Podman));
                Response::success(serde_json::json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"native":"http","docker":docker,"podman":podman,"ai":"not_implemented"}))
            },
            Command::RuntimeCatalog { runtime } => Response::success(state.runtimes.catalog(&runtime).await?),
            Command::RuntimeInstalled => Response::success(state.runtimes.installed().await?),
            Command::RuntimeInstall { runtime,version } => Response::success(state.install(runtime,version).await?),
            Command::RuntimeDiscover => Response::success(state.runtimes.discover()),
            Command::RuntimeUseSystem { runtime,executable } => Response::success(state.runtimes.use_system(&runtime,&executable).await?),
            Command::ProjectDetect {root} => Response::success(dockon_runtime_manager::projects::detect(&root)?),
            Command::ProjectCreate {id,name,root,runtime_installation} => {
                if state.projects.list().await?.iter().any(|p|p.manifest.id==id){anyhow::bail!("Project id is already registered");}
                let manifest=dockon_runtime_manager::projects::create(&state.runtimes,&id,&name,&root,&runtime_installation).await?;
                Response::success(state.projects.register(manifest).await?)
            },
            Command::ProjectImport {id,name,root,runtime_installation,profile,php_cgi_installation} => {
                let mut manifest=dockon_runtime_manager::projects::import(&state.runtimes,&id,&name,&root,&runtime_installation,&profile).await?;
                dockon_runtime_manager::projects::attach_php_cgi(&state.runtimes, &mut manifest, &profile, php_cgi_installation.as_deref()).await?;
                Response::success(state.projects.register(manifest).await?)
            },
            Command::ProjectSwitchStaticServer { id, runtime_installation } => Response::success(
                state.projects.submit_static_switch(id, runtime_installation).await?
            ),
            Command::List => Response::success(state.projects.list().await?),
            Command::ProjectAddRedis { id } => Response::success(state.projects.add_redis(&id).await?),
            Command::ProjectAddMailpit { id } => Response::success(state.projects.add_mailpit(&id).await?),
            Command::ProjectAddPostgres { id } => Response::success(state.projects.add_postgres(&id).await?),
            Command::ProjectAddDatabase { id, engine } => Response::success(state.projects.add_database(&id, &engine).await?),
            Command::PostgresConnection { id } => Response::success(state.projects.postgres_connection(&id).await?),
            Command::DatabaseConnection { id, engine } => Response::success(state.projects.database_connection(&id, &engine).await?),
            Command::PostgresBackup { id } => Response::success(state.projects.backup_postgres(&id).await?),
            Command::PostgresBackups { id } => Response::success(state.projects.postgres_backups(&id).await?),
            Command::PostgresRestore { id, backup } => Response::success(state.projects.submit_postgres_restore(id, backup).await?),
            Command::Register { manifest_path } => Response::success(state.projects.register(Manifest::read(&manifest_path)?).await?),
            Command::Start { id } => Response::success(state.projects.submit(id, true).await?),
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
