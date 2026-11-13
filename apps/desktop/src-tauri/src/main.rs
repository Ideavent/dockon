#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tauri::command]
async fn request(command: dockon_core::Command) -> Result<dockon_core::Response, String> {
    dockon_ipc::call(command)
        .await
        .map_err(|error| format!("{error:#}"))
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
        .invoke_handler(tauri::generate_handler![request, connect])
        .run(tauri::generate_context!())
        .expect("Dockon desktop failed");
}
