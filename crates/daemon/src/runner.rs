// Internal Unix process supervisor. Its parent owns the stdin pipe, so daemon
// death is detected without trusting a possibly reused parent PID.
#[cfg(unix)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    use std::os::fd::{AsRawFd, FromRawFd};
    use tokio::signal::unix::{signal, SignalKind};
    let mut args = std::env::args_os().skip(1);
    let executable = args
        .next()
        .context("Internal runner requires an executable")?;
    let mut term = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut command = tokio::process::Command::new(executable);
    command
        .args(args)
        .process_group(0)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn()?;
    let group = child.id().context("Child PID unavailable")? as i32;
    // AsyncFd avoids an uncancellable blocking stdin read keeping the Tokio
    // runtime alive after the application has already exited.
    let fd = unsafe { libc::dup(libc::STDIN_FILENO) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let pipe = unsafe { std::fs::File::from_raw_fd(fd) };
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: this File exclusively owns the descriptor; it is never replaced
    // or closed while registered with AsyncFd.
    let parent = unsafe { tokio::io::unix::AsyncFd::register(pipe)? };
    let early = tokio::select! {
        status=child.wait()=>Some(status?),
        _=parent_closed(&parent)=>None,
        _=term.recv()=>None,
        _=interrupt.recv()=>None,
    };
    if let Some(status) = early {
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
    } else {
        unsafe {
            libc::kill(-group, libc::SIGTERM);
        }
        if tokio::time::timeout(std::time::Duration::from_secs(4), child.wait())
            .await
            .is_err()
        {
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        // Also terminate descendants that outlived a gracefully exiting parent.
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
        let _ = child.wait().await?;
    }
    Ok(())
}

#[cfg(unix)]
async fn parent_closed(pipe: &tokio::io::unix::AsyncFd<std::fs::File>) -> std::io::Result<()> {
    use std::io::Read;
    loop {
        let mut ready = pipe.readable().await?;
        match ready.try_io(|inner| {
            let mut byte = [0u8; 1];
            let _ = inner.get_ref().read(&mut byte)?;
            Ok(())
        }) {
            Ok(result) => return result,
            Err(_) => continue,
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("dockon-runner is an internal Unix helper; Windows uses Job Objects");
}
