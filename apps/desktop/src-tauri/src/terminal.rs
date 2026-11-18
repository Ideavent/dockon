use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::null,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, WAIT_OBJECT_0},
    System::Threading::{
        CreateProcessW, GetExitCodeProcess, WaitForSingleObject, CREATE_NEW_CONSOLE,
        CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTF_USESHOWWINDOW, STARTUPINFOW,
    },
};

pub fn open(
    root: &Path,
    path: &OsStr,
    tools: &std::collections::BTreeMap<String, String>,
) -> Result<u32, String> {
    let system = std::env::var_os("SystemRoot").ok_or("Windows directory unavailable")?;
    let cmd = PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let executable = wide(cmd.as_os_str());
    let mut command = OsString::from("\"");
    command.push(&cmd);
    command.push("\" -NoLogo -NoProfile -NoExit");
    if tools.contains_key("DOCKON_COMPOSER") || tools.contains_key("DOCKON_SDKMANAGER") {
        command.push(" -Command \"");
        if tools.contains_key("DOCKON_COMPOSER") {
            command.push("function global:composer { $phpOptions=@(); if ((Test-Path env:DOCKON_COMPOSER_INI)) { $phpOptions=@('-c',(Get-Item env:DOCKON_COMPOSER_INI).Value) }; & (Get-Item env:DOCKON_COMPOSER_PHP).Value @phpOptions (Get-Item env:DOCKON_COMPOSER).Value @args }; ");
        }
        if tools.contains_key("DOCKON_SDKMANAGER") {
            command.push("function global:sdkmanager { & (Get-Item env:DOCKON_SDKMANAGER).Value ('--sdk_root='+(Get-Item env:ANDROID_HOME).Value) @args }; ");
        }
        command.push("\"");
    }
    let mut command = wide(&command);
    let directory = wide(root.as_os_str());
    let environment = environment(root, path, tools);
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        dwFlags: STARTF_USESHOWWINDOW,
        wShowWindow: 1,
        ..STARTUPINFOW::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // Give the new console its own standard handles when Dockon has redirected I/O.
    let started = unsafe {
        CreateProcessW(
            executable.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            directory.as_ptr(),
            &startup,
            &mut process,
        )
    };
    if started == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let exit_code = unsafe {
        CloseHandle(process.hThread);
        let mut exit_code = 0;
        let exited = WaitForSingleObject(process.hProcess, 250) == WAIT_OBJECT_0
            && GetExitCodeProcess(process.hProcess, &mut exit_code) != 0;
        CloseHandle(process.hProcess);
        exited.then_some(exit_code)
    };
    if let Some(exit_code) = exit_code {
        return Err(format!(
            "Project terminal exited during startup ({exit_code})"
        ));
    }
    Ok(process.dwProcessId)
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn environment(
    root: &Path,
    path: &OsStr,
    tools: &std::collections::BTreeMap<String, String>,
) -> Vec<u16> {
    let mut entries: Vec<_> = std::env::vars_os()
        .filter(|(key, _)| {
            !tools
                .keys()
                .any(|name| key.to_string_lossy().eq_ignore_ascii_case(name))
                && !["PATH", "DOCUMENT_ROOT"]
                    .iter()
                    .any(|name| key.to_string_lossy().eq_ignore_ascii_case(name))
        })
        .collect();
    entries.extend([
        (OsString::from("PATH"), path.to_os_string()),
        (
            OsString::from("DOCUMENT_ROOT"),
            root.as_os_str().to_os_string(),
        ),
    ]);
    entries.extend(tools.iter().map(|(key, value)| (key.into(), value.into())));
    entries.sort_by_cached_key(|(key, _)| key.to_string_lossy().to_uppercase());
    let mut block = Vec::new();
    for (key, value) in entries {
        block.extend(key.encode_wide());
        block.push(u16::from(b'='));
        block.extend(wide(&value));
    }
    block.push(0);
    block
}
