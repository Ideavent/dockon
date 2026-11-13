use anyhow::{bail, Context, Result};
use dockon_core::{Command, Request, Response};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

#[derive(Clone, Serialize, Deserialize)]
pub struct Connection {
    pub endpoint: String,
    pub token: String,
}

pub fn data_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("DOCKON_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    Ok(directories::ProjectDirs::from("dev", "dockon", "dockon")
        .context("No user data directory")?
        .data_local_dir()
        .into())
}

pub fn prepare_private_dir() -> Result<PathBuf> {
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let who = std::process::Command::new("whoami.exe")
            .arg("/user")
            .arg("/fo")
            .arg("csv")
            .arg("/nh")
            .creation_flags(0x08000000)
            .output()?;
        let text = String::from_utf8(who.stdout)?;
        let sid = text
            .trim()
            .split(',')
            .next_back()
            .context("Missing user SID")?
            .trim_matches('"');
        if !sid.starts_with("S-1-") {
            bail!("Unable to determine current user SID");
        }
        let status = std::process::Command::new("icacls.exe")
            .arg(&dir)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("*{sid}:(OI)(CI)F"))
            .creation_flags(0x08000000)
            .stdout(std::process::Stdio::null())
            .status()?;
        if !status.success() {
            bail!("Unable to secure Dockon data directory");
        }
    }
    Ok(dir)
}

pub async fn read_json<T: serde::de::DeserializeOwned>(
    stream: impl AsyncRead + Unpin,
) -> Result<T> {
    let mut reader = BufReader::new(stream);
    read_frame(&mut reader).await
}

pub async fn read_frame<T: serde::de::DeserializeOwned, R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<T> {
    let mut line = Vec::new();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            bail!("Connection closed before response");
        }
        let end = chunk.iter().position(|b| *b == b'\n');
        let n = end.map_or(chunk.len(), |i| i + 1);
        if line.len() + n > 1024 * 1024 {
            bail!("IPC frame exceeds 1 MiB");
        }
        line.extend_from_slice(&chunk[..n]);
        reader.consume(n);
        if end.is_some() {
            return Ok(serde_json::from_slice(&line)?);
        }
    }
}

pub async fn write_json(
    stream: &mut (impl AsyncWrite + Unpin),
    value: &impl Serialize,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn subscribe(
    after: u64,
    mut receive: impl FnMut(dockon_core::EventBatch) -> Result<()>,
) -> Result<()> {
    let connection: Connection =
        serde_json::from_slice(&std::fs::read(data_dir()?.join("connection.json"))?)?;
    #[cfg(windows)]
    let mut stream = connect_pipe(&connection.endpoint).await?;
    #[cfg(unix)]
    let mut stream = tokio::net::UnixStream::connect(&connection.endpoint).await?;
    write_json(
        &mut stream,
        &Request {
            version: 1,
            token: connection.token,
            command: Command::Subscribe { after },
        },
    )
    .await?;
    let mut reader = BufReader::new(stream);
    loop {
        let response: Response =
            tokio::time::timeout(std::time::Duration::from_secs(5), read_frame(&mut reader))
                .await??;
        if !response.ok {
            bail!("{}", response.error.unwrap_or_default());
        }
        receive(serde_json::from_value(response.data)?)?;
    }
}

pub async fn call(command: Command) -> Result<Response> {
    let connection: Connection = serde_json::from_slice(
        &std::fs::read(data_dir()?.join("connection.json")).context("Daemon is not available")?,
    )?;
    call_with(&connection, command).await
}
pub async fn call_with(connection: &Connection, command: Command) -> Result<Response> {
    let request = Request {
        version: 1,
        token: connection.token.clone(),
        command,
    };
    tokio::time::timeout(std::time::Duration::from_secs(35), async {
        #[cfg(windows)]
        let mut stream = connect_pipe(&connection.endpoint).await?;
        #[cfg(unix)]
        let mut stream = tokio::net::UnixStream::connect(&connection.endpoint).await?;
        write_json(&mut stream, &request).await?;
        let response: Response = read_json(&mut stream).await?;
        if response.version != 1 {
            bail!("Unsupported IPC response version");
        }
        Ok(response)
    })
    .await
    .context("Daemon request timed out")?
}

#[cfg(windows)]
async fn connect_pipe(endpoint: &str) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    // Retry connection only, never an RPC that may already have executed.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
            Ok(stream) => return Ok(stream),
            Err(error)
                if matches!(error.raw_os_error(), Some(231) | Some(2))
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn preserves_coalesced_subscription_frames() {
        let (mut writer, reader) = tokio::io::duplex(128);
        tokio::spawn(async move {
            writer.write_all(b"{\"n\":1}\n{\"n\":2}\n").await.unwrap();
        });
        let mut reader = BufReader::new(reader);
        let first: serde_json::Value = read_frame(&mut reader).await.unwrap();
        let second: serde_json::Value = read_frame(&mut reader).await.unwrap();
        assert_eq!(first["n"], 1);
        assert_eq!(second["n"], 2);
    }
    #[tokio::test]
    async fn rejects_truncated_and_oversized_frames() {
        let (mut writer, reader) = tokio::io::duplex(128);
        tokio::spawn(async move {
            writer.write_all(b"{\"ok\":true}").await.unwrap();
        });
        assert!(read_json::<serde_json::Value>(reader).await.is_err());
        let (mut writer, reader) = tokio::io::duplex(8192);
        tokio::spawn(async move {
            let bytes = vec![b'x'; 1024 * 1024 + 1];
            let _ = writer.write_all(&bytes).await;
        });
        assert!(read_json::<serde_json::Value>(reader)
            .await
            .unwrap_err()
            .to_string()
            .contains("exceeds"));
    }
}
