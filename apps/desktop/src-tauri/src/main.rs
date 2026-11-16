#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod terminal;

use tauri::{Emitter, Manager};

#[derive(Default)]
struct EventStream(std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>);

#[tauri::command]
fn watch_events(
    channel: tauri::ipc::Channel<dockon_core::EventBatch>,
    state: tauri::State<EventStream>,
) -> Result<(), String> {
    let mut stream = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(old) = stream.take() {
        old.abort();
    }
    *stream = Some(tauri::async_runtime::spawn(async move {
        loop {
            let result =
                dockon_ipc::subscribe(0, |batch| channel.send(batch).map_err(Into::into)).await;
            if result.is_ok() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }));
    Ok(())
}

#[tauri::command]
fn stop_events(state: tauri::State<EventStream>) -> Result<(), String> {
    if let Some(old) = state.0.lock().map_err(|e| e.to_string())?.take() {
        old.abort();
    }
    Ok(())
}

#[tauri::command]
async fn request(command: dockon_core::Command) -> Result<dockon_core::Response, String> {
    dockon_ipc::call(command)
        .await
        .map_err(|error| format!("{error:#}"))
}

async fn domain_resolves(domain: &str, port: u16, dedicated: bool) -> bool {
    let addresses = tokio::time::timeout(
        std::time::Duration::from_millis(700),
        tokio::net::lookup_host((domain, port)),
    )
    .await;
    let valid = |address: std::net::SocketAddr| {
        if dedicated {
            address.ip() == std::net::IpAddr::V4(dockon_core::WINDOWS_WEBMAIL_IP)
        } else {
            address.ip().is_loopback()
        }
    };
    addresses.is_ok_and(|result| {
        result
            .is_ok_and(|mut addresses| addresses.next().is_some_and(valid) && addresses.all(valid))
    })
}

async fn register_local_domains() -> Result<(), String> {
    use dockon_core::Command;
    static HOSTS_UPDATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = HOSTS_UPDATE.lock().await;
    let status = request(Command::RouterStatus).await?;
    if status.data["hosts_pending"].as_bool() != Some(true) {
        return Ok(());
    }
    let plan = request(Command::SystemPlan {
        action: dockon_core::SystemAction::HostsUpdate,
    })
    .await?;
    if !plan.ok {
        return Err(plan
            .error
            .unwrap_or_else(|| "Unable to prepare local addresses".into()));
    }
    if plan.data["before"] == plan.data["after"] {
        return Ok(());
    }
    let token = plan.data["token"]
        .as_str()
        .ok_or("Missing hosts plan")?
        .to_owned();
    let result = request(Command::SystemApply { token }).await?;
    if !result.ok {
        return Err(result
            .error
            .unwrap_or_else(|| "Unable to apply local addresses".into()));
    }
    let id = result.data["id"]
        .as_str()
        .ok_or("Missing hosts operation")?
        .to_owned();
    for _ in 0..600 {
        let reply = request(Command::Operation { id: id.clone() }).await?;
        if !reply.ok {
            return Err(reply
                .error
                .unwrap_or_else(|| "Unable to check local addresses".into()));
        }
        match reply.data["status"].as_str() {
            Some("completed") => return Ok(()),
            Some("running") => tokio::time::sleep(std::time::Duration::from_millis(300)).await,
            _ => {
                return Err(reply.data["error"]
                    .as_str()
                    .unwrap_or("Local addresses could not be registered")
                    .into())
            }
        }
    }
    Err("Windows administrator approval is still pending".into())
}

#[tauri::command]
async fn prepare_project_address(id: String) -> Result<(), String> {
    let reply = request(dockon_core::Command::WorkspaceEnsureRoute { id }).await?;
    if !reply.ok {
        return Err(reply
            .error
            .unwrap_or_else(|| "Unable to prepare the project address".into()));
    }
    if cfg!(windows) && reply.data["running"].as_bool() == Some(true) {
        register_local_domains().await?;
    }
    Ok(())
}

#[tauri::command]
async fn open_project(
    app: tauri::AppHandle,
    id: String,
    inbox: Option<bool>,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let project = get_project(&id).await?;
    let inbox = inbox.unwrap_or(false);
    let port = if inbox {
        project
            .services
            .iter()
            .find(|service| {
                service.id == "mailpit"
                    && service.state == dockon_core::State::Running
                    && service.smtp_port.is_some()
            })
            .ok_or("Mailpit is not ready")?
            .port
    } else {
        if project.state != dockon_core::State::Running
            || matches!(project.manifest.health, dockon_core::HealthCheck::Tcp)
        {
            return Err("Project does not have a ready HTTP endpoint".into());
        }
        project.port.ok_or("Project port unavailable")?
    };
    if cfg!(windows) {
        prepare_project_address(id.clone()).await?;
    }
    let mut url = format!("http://127.0.0.1:{port}");
    if let Ok(response) = dockon_ipc::call(dockon_core::Command::RouterStatus).await {
        if response.ok && response.data["running"].as_bool() == Some(true) {
            if let Ok(settings) = serde_json::from_value::<dockon_core::RouterSettings>(
                response.data["settings"].clone(),
            ) {
                if let Some(route) = settings.routes.iter().find(|route| route.project_id == id) {
                    let (scheme, port, default_port) = if route.https && !inbox {
                        ("https", settings.https_port, 443)
                    } else {
                        ("http", settings.http_port, 80)
                    };
                    let domain = if inbox {
                        format!("webmail.{}", route.domain)
                    } else {
                        route.domain.clone()
                    };
                    let dedicated = cfg!(windows) && !domain.ends_with(".localhost");
                    let canonical_port = if dedicated { default_port } else { port };
                    let local_domain = domain_resolves(&domain, canonical_port, dedicated).await;
                    if dedicated && !local_domain {
                        return Err(format!(
                            "{domain} does not resolve to Dockon. Check its Windows hosts mapping."
                        ));
                    }
                    let target = if local_domain {
                        Some((domain, canonical_port))
                    } else if inbox {
                        Some((settings.webmail_alias(route, &project), port))
                    } else {
                        settings
                            .workspace_alias(route, &project)
                            .map(|host| (host, port))
                    };
                    if let Some((host, port)) = target {
                        url = format!("{scheme}://{host}");
                        if port != default_port {
                            url.push_str(&format!(":{port}"));
                        }
                    }
                }
            }
        }
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn connect() -> Result<dockon_core::Response, String> {
    static STARTUP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = STARTUP.lock().await;
    if let Ok(response) = dockon_ipc::call(dockon_core::Command::Health).await {
        return Ok(response);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let daemon = exe
        .parent()
        .ok_or("Missing application directory")?
        .join(if cfg!(windows) {
            "dockon-daemon.exe"
        } else {
            "dockon-daemon"
        });
    if !daemon.is_file() {
        return Err("Daemon executable is missing next to the desktop application".into());
    }
    let mut process = std::process::Command::new(daemon);
    process
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x08000000);
    }
    let mut child = process.spawn().map_err(|e| e.to_string())?;
    // Reap after shutdown without coupling daemon lifetime to the UI.
    let (exited, status) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = exited.send(child.wait());
    });
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(25);
    tokio::pin!(status);
    while tokio::time::Instant::now() < deadline {
        if let Ok(response) = dockon_ipc::call(dockon_core::Command::Health).await {
            return Ok(response);
        }
        tokio::select! {
            result = &mut status => {
                let code = result.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
                return Err(format!("Daemon exited during startup ({code})"));
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
        }
    }
    Err("Daemon startup timed out; inspect daemon logs".into())
}

async fn checked_call(command: dockon_core::Command) -> Result<serde_json::Value, String> {
    let reply = request(command).await?;
    if reply.ok {
        Ok(reply.data)
    } else {
        Err(reply
            .error
            .unwrap_or_else(|| "Dockon request failed".into()))
    }
}

async fn get_project(id: &str) -> Result<dockon_core::Project, String> {
    let projects: Vec<dockon_core::Project> =
        serde_json::from_value(checked_call(dockon_core::Command::List).await?)
            .map_err(|error| error.to_string())?;
    projects
        .into_iter()
        .find(|project| project.manifest.id == id)
        .ok_or_else(|| "Unknown project".into())
}

#[tauri::command]
async fn open_workspace(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let workspace = checked_call(dockon_core::Command::WorkspaceFolders).await?;
    let root = workspace["root"].as_str().ok_or("Missing www directory")?;
    std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
    app.opener()
        .open_path(root, None::<&str>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn export_project_preset(
    app: tauri::AppHandle,
    id: String,
    name: String,
) -> Result<String, String> {
    use tauri_plugin_opener::OpenerExt;
    let value = checked_call(dockon_core::Command::ProjectPresetCapture { id, name }).await?;
    let clock = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let exports = dockon_ipc::data_dir()
        .map_err(|error| error.to_string())?
        .join("exports");
    std::fs::create_dir_all(&exports).map_err(|error| error.to_string())?;
    let directory = exports.join(clock.to_string());
    std::fs::create_dir(&directory).map_err(|error| error.to_string())?;
    let path = directory.join("dockon.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    app.opener()
        .open_path(directory.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
async fn project_terminal(id: String) -> Result<u32, String> {
    use std::path::PathBuf;
    let project = get_project(&id).await?;
    let bins: Vec<PathBuf> = serde_json::from_value(
        checked_call(dockon_core::Command::ProjectRuntimePaths { id }).await?,
    )
    .map_err(|error| error.to_string())?;
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let path = std::env::join_paths(bins.into_iter().chain(std::env::split_paths(&inherited)))
        .map_err(|error| error.to_string())?;
    let tools: std::collections::BTreeMap<String, String> = serde_json::from_value(
        checked_call(dockon_core::Command::ProjectToolEnvironment {
            id: project.manifest.id.clone(),
        })
        .await?,
    )
    .map_err(|error| error.to_string())?;
    let root = project.manifest.root;
    #[cfg(windows)]
    {
        terminal::open(&root, &path, &tools)
    }
    #[cfg(not(windows))]
    {
        let mut child = std::process::Command::new("x-terminal-emulator")
            .current_dir(&root)
            .env("DOCUMENT_ROOT", &root)
            .env("PATH", path)
            .envs(tools)
            .spawn()
            .map_err(|error| error.to_string())?;
        let pid = child.id();
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(pid)
    }
}

fn show_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let separator = PredefinedMenuItem::separator(app)?;
    let menu = MenuBuilder::new(app)
        .text("show", "Dockon’u aç")
        .item(&separator)
        .text("www", "www klasörü")
        .separator()
        .text("quit", "Servisleri durdur ve çık")
        .build()?;
    let tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Dockon")
        .show_menu_on_left_click(false);
    let tray = if let Some(icon) = app.default_window_icon() {
        tray.icon(icon.clone())
    } else {
        tray
    };
    tray.on_tray_icon_event(|tray, event| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            show_window(tray.app_handle());
        }
    })
    .on_menu_event(|app, event| {
        let action = event.id().as_ref().to_owned();
        if action == "show" {
            show_window(app);
            return;
        }
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            use dockon_core::Command;
            let result = match action.as_str() {
                "www" => open_workspace(app.clone()).await,
                "quit" => {
                    if dockon_ipc::call(Command::Health).await.is_err() {
                        app.exit(0);
                        return;
                    }
                    async {
                        let connection = dockon_ipc::data_dir()
                            .map_err(|error| error.to_string())?
                            .join("connection.json");
                        checked_call(Command::Shutdown).await?;
                        for _ in 0..180 {
                            if !connection.exists() {
                                app.exit(0);
                                return Ok(());
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        }
                        Err("Daemon shutdown did not finish".to_owned())
                    }
                    .await
                }
                _ => return,
            };
            if let Err(error) = &result {
                show_window(&app);
                let _ = app.emit("desktop-error", error);
            }
        });
    })
    .build(app)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(EventStream::default())
        .setup(|app| {
            setup_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            request,
            connect,
            watch_events,
            stop_events,
            open_project,
            prepare_project_address,
            open_workspace,
            export_project_preset,
            project_terminal
        ])
        .run(tauri::generate_context!())
        .expect("Dockon desktop failed");
}
