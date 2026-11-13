use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--worker") {
        std::future::pending::<()>().await;
    }
    if let Some(index) = args.iter().position(|arg| arg == "--startup-delay-ms") {
        let delay: u64 = args
            .get(index + 1)
            .ok_or_else(|| anyhow::anyhow!("Delay required"))?
            .parse()?;
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }
    let port: u16 = std::env::var("PORT")?.parse()?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("Dockon sample listening on 127.0.0.1:{port}");
    let mut worker_started = false;
    loop {
        let (mut socket, _) = listener.accept().await?;
        // Optional sample worker allows validating real process-tree cleanup.
        if !worker_started && args.iter().any(|arg| arg == "--with-worker") {
            let child = std::process::Command::new(std::env::current_exe()?)
                .arg("--worker")
                .spawn()?;
            println!("worker_pid={}", child.id());
            worker_started = true;
        }
        tokio::spawn(async move {
            let mut request = [0; 8192];
            if let Ok(Ok(n)) =
                tokio::time::timeout(std::time::Duration::from_secs(2), socket.read(&mut request))
                    .await
            {
                println!(
                    "{}",
                    String::from_utf8_lossy(&request[..n])
                        .lines()
                        .next()
                        .unwrap_or("request")
                );
                let body = "{\"name\":\"dockon-hello-http\",\"status\":\"ok\"}";
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
    }
}
