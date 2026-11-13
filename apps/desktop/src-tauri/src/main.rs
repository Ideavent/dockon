#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

#[tauri::command]
async fn open_project(app: tauri::AppHandle, id: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let response = dockon_ipc::call(dockon_core::Command::List)
        .await
        .map_err(|e| e.to_string())?;
    if !response.ok {
        return Err(response.error.unwrap_or_default());
    }
    let projects: Vec<dockon_core::Project> =
        serde_json::from_value(response.data).map_err(|e| e.to_string())?;
    let project = projects
        .into_iter()
        .find(|p| p.manifest.id == id)
        .ok_or("Unknown project")?;
    if project.state != dockon_core::State::Running
        || matches!(project.manifest.health, dockon_core::HealthCheck::Tcp)
    {
        return Err("Project does not have a ready HTTP endpoint".into());
    }
    let port = project.port.ok_or("Project port unavailable")?;
    app.opener()
        .open_url(format!("http://127.0.0.1:{port}"), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn connect() -> Result<dockon_core::Response, String> {
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
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Ok(response) = dockon_ipc::call(dockon_core::Command::Health).await {
            return Ok(response);
        }
    }
    Err("Daemon startup timed out; inspect daemon logs".into())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(EventStream::default())
        .invoke_handler(tauri::generate_handler![
            request,
            connect,
            watch_events,
            stop_events,
            open_project
        ])
        .run(tauri::generate_context!())
        .expect("Dockon desktop failed");
}
