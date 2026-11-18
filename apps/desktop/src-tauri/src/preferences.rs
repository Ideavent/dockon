use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    #[default]
    Ask,
    Minimize,
    Quit,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Preferences {
    close_behavior: CloseBehavior,
}

#[derive(Serialize)]
pub struct DesktopPreferences {
    close_behavior: CloseBehavior,
    auto_start: Option<bool>,
    auto_start_error: Option<String>,
}

fn preferences_path() -> Result<PathBuf, String> {
    Ok(dockon_ipc::data_dir()
        .map_err(|error| error.to_string())?
        .join("desktop-preferences.json"))
}

fn load_preferences() -> Result<Preferences, String> {
    match std::fs::read(preferences_path()?) {
        Ok(data) => serde_json::from_slice(&data).map_err(|error| error.to_string()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn close_behavior() -> CloseBehavior {
    load_preferences().unwrap_or_default().close_behavior
}

#[tauri::command]
pub fn desktop_preferences(app: tauri::AppHandle) -> DesktopPreferences {
    let (auto_start, auto_start_error) = match read_auto_start(&app) {
        Ok(value) => (value, None),
        Err(error) => (None, Some(error)),
    };
    DesktopPreferences {
        close_behavior: close_behavior(),
        auto_start,
        auto_start_error,
    }
}

#[tauri::command]
pub fn set_close_behavior(behavior: CloseBehavior) -> Result<CloseBehavior, String> {
    let path = preferences_path()?;
    let directory = path.parent().ok_or("Missing preferences directory")?;
    let mut preferences = load_preferences().unwrap_or_default();
    preferences.close_behavior = behavior;
    let data = serde_json::to_vec(&preferences).map_err(|error| error.to_string())?;
    let temporary = directory.join(format!("desktop-preferences-{}.tmp", std::process::id()));
    let save = || -> io::Result<()> {
        std::fs::create_dir_all(directory)?;
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, &path)
    };
    if let Err(error) = save() {
        let _ = std::fs::remove_file(temporary);
        return Err(error.to_string());
    }
    Ok(behavior)
}

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "Dockon";

#[cfg(windows)]
fn read_auto_start(_app: &tauri::AppHandle) -> Result<Option<bool>, String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let value = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>(RUN_VALUE));
    match value {
        Ok(command) => Ok(Some(!command.is_empty())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Some(false)),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(target_os = "macos")]
fn read_auto_start(app: &tauri::AppHandle) -> Result<Option<bool>, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch()
        .is_enabled()
        .map(Some)
        .map_err(|error| error.to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn read_auto_start(_app: &tauri::AppHandle) -> Result<Option<bool>, String> {
    Ok(None)
}

#[tauri::command]
pub fn set_auto_start(_app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    #[cfg(windows)]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStrExt};
        use winreg::{
            enums::{HKEY_CURRENT_USER, KEY_SET_VALUE},
            RegKey,
        };

        let user = RegKey::predef(HKEY_CURRENT_USER);
        if enabled {
            let exe = std::env::current_exe().map_err(|error| error.to_string())?;
            let mut command = OsString::from("\"");
            command.push(exe);
            command.push("\"");
            if command.encode_wide().count() > 260 {
                return Err("The executable path is too long for Windows startup".into());
            }
            let (key, _) = user
                .create_subkey(RUN_KEY)
                .map_err(|error| error.to_string())?;
            key.set_value(RUN_VALUE, &command)
                .map_err(|error| error.to_string())?;
        } else if let Err(error) = user
            .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
            .and_then(|key| key.delete_value(RUN_VALUE))
        {
            if error.kind() != io::ErrorKind::NotFound {
                return Err(error.to_string());
            }
        }
        Ok(enabled)
    }
    #[cfg(target_os = "macos")]
    {
        use tauri_plugin_autostart::ManagerExt;
        let manager = _app.autolaunch();
        let result = if enabled {
            manager.enable()
        } else {
            manager.disable()
        };
        result.map_err(|error| error.to_string())?;
        Ok(enabled)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = enabled;
        Err("Auto-start is available on Windows and macOS".into())
    }
}
