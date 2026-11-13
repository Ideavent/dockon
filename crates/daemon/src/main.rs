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
    let orchestrator = Orchestrator::new(store, dir.clone()).await?;
    let events = orchestrator.events.clone();
    let state = Arc::new(orchestrator);
    let shutdown = Arc::new(Notify::new());
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
                let state = state.clone(); let token = token.clone(); let events = events.clone(); let shutdown = shutdown.clone();
                tokio::spawn(async move { if let Err(error) = serve(stream, token, state, events, shutdown).await { tracing::warn!(%error, "IPC connection failed"); } });
            }
        }
    }
    state.shutdown().await?;
    std::fs::remove_file(dir.join("connection.json"))?;
    #[cfg(unix)]
    std::fs::remove_file(endpoint)?;
    tracing::info!("Dockon daemon stopped");
    Ok(())
}

async fn serve(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    token: String,
    state: Arc<Orchestrator>,
    events: EventBus,
    shutdown: Arc<Notify>,
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
            let batch = events.lock().await.batch(cursor);
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
            Command::Health => Response::success(serde_json::json!({"name":"dockon", "version":env!("CARGO_PKG_VERSION"), "native":"http_tcp", "container":"digest_pinned_http_tcp", "ai":"not_implemented"})),
            Command::Capabilities => {
                let (docker,podman)=tokio::join!(dockon_provider_container::capability(dockon_core::ContainerEngine::Docker),dockon_provider_container::capability(dockon_core::ContainerEngine::Podman));
                Response::success(serde_json::json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"native":"http","docker":docker,"podman":podman,"ai":"not_implemented"}))
            },
            Command::List => Response::success(state.list().await?),
            Command::Register { manifest_path } => Response::success(state.register(Manifest::read(&manifest_path)?).await?),
            Command::Start { id } => Response::success(state.submit(id, true).await?),
            Command::Stop { id } => Response::success(state.submit(id, false).await?),
            Command::Operation { id } => Response::success(state.operation(&id).await?),
            Command::Events { after } => {
                let feed = events.lock().await;
                Response::success(feed.batch(after))
            },
            Command::Subscribe { .. } => unreachable!(),
            Command::Shutdown => {
                shutdown.notify_one();
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
