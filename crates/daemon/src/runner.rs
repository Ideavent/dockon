// Internal Unix process supervisor. Its parent owns the stdin pipe, so daemon
// death is detected without trusting a possibly reused parent PID.
#[cfg(unix)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use anyhow::Context;
    use tokio::{
        io::AsyncReadExt,
        signal::unix::{signal, SignalKind},
    };
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
    let mut parent = tokio::io::stdin();
    let mut byte = [0u8; 1];
    let early = tokio::select! {
        status=child.wait()=>Some(status?),
        _=parent.read(&mut byte)=>None,
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

#[cfg(not(unix))]
fn main() {
    eprintln!("dockon-runner is an internal Unix helper; Windows uses Job Objects");
}
