use anyhow::{bail, Result};
use dockon_core::Manifest;
use std::process::Stdio;
use tokio::process::{Child, Command};

pub struct Instance {
    pub owner: String,
    pub child: Child,
    #[cfg(unix)]
    group: i32,
    #[cfg(windows)]
    job: Job,
}
#[cfg(windows)]
struct Job(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
unsafe impl Send for Job {}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

impl Instance {
    pub fn spawn(
        manifest: &Manifest,
        port: u16,
        data_dir: &std::path::Path,
        environment: &std::collections::BTreeMap<String, String>,
    ) -> Result<Self> {
        manifest.validate()?;
        #[cfg(windows)]
        let mut command = Command::new(&manifest.executable);
        #[cfg(unix)]
        let mut command = {
            let executable = std::env::current_exe()?;
            let runner = executable
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Daemon executable directory unavailable"))?
                .join("dockon-runner");
            if !runner.is_file() {
                bail!("Internal dockon-runner helper is missing");
            }
            let mut command = Command::new(runner);
            command.arg(&manifest.executable);
            command
        };
        command
            .args(
                manifest
                    .args
                    .iter()
                    .map(|arg| arg.resolve(port, data_dir))
                    .collect::<Result<Vec<_>>>()?,
            )
            .current_dir(&manifest.root)
            .env("PORT", port.to_string())
            .env("HOST", "127.0.0.1")
            .env("DOCKON_PROJECT_DATA_DIR", data_dir)
            .envs(environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if manifest.runtime.is_some() {
            command
                .env_remove("NODE_OPTIONS")
                .env_remove("NODE_PATH")
                .env_remove("PYTHONHOME")
                .env_remove("PYTHONPATH");
        }
        #[cfg(unix)]
        command
            .process_group(0)
            .stdin(Stdio::piped())
            .kill_on_drop(false);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let child = command.spawn()?;
        #[cfg(windows)]
        let mut child = child;
        #[cfg(windows)]
        let job = unsafe {
            use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                child.start_kill()?;
                return Err(std::io::Error::last_os_error().into());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as _,
                std::mem::size_of_val(&info) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.raw_handle().unwrap() as _) == 0
            {
                let error = std::io::Error::last_os_error();
                CloseHandle(handle);
                child.start_kill()?;
                return Err(error.into());
            }
            Job(handle)
        };
        Ok(Self {
            owner: manifest.id.clone(),
            #[cfg(unix)]
            group: child.id().expect("newly spawned child has a PID") as i32,
            child,
            #[cfg(windows)]
            job,
        })
    }
    pub async fn stop(&mut self, owner: &str) -> Result<()> {
        if owner != self.owner {
            bail!("Process ownership mismatch");
        }
        #[cfg(windows)]
        unsafe {
            if windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job.0, 1) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(unix)]
        {
            {
                unsafe {
                    libc::kill(-self.group, libc::SIGTERM);
                }
                if tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait())
                    .await
                    .is_err()
                {
                    unsafe {
                        libc::kill(-self.group, libc::SIGKILL);
                    }
                }
            }
        }
        let _ = self.child.wait().await?;
        Ok(())
    }
}

#[cfg(unix)]
impl Drop for Instance {
    fn drop(&mut self) {
        // Let the runner kill/reap its application group on private pipe EOF.
        self.child.stdin.take();
    }
}
