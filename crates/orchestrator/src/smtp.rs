use anyhow::{Context, Result};
use std::time::Duration;
use tokio::{
    io,
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

pub struct Forward(JoinHandle<()>);

impl Forward {
    pub async fn start(project: &str, upstream: u16) -> Result<Self> {
        let address = dockon_core::project_smtp_ip(project);
        let listener = TcpListener::bind((address, 25))
            .await
            .with_context(|| format!("Cannot open the project's SMTP listener at {address}:25"))?;
        Ok(Self(tokio::spawn(async move {
            let mut clients = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept(), if clients.len() < 64 => {
                        let Ok((mut connection, _)) = accepted else { break };
                        clients.spawn(async move {
                            let _ = tokio::time::timeout(Duration::from_secs(300), async {
                                let mut server = TcpStream::connect(("127.0.0.1", upstream)).await?;
                                io::copy_bidirectional(&mut connection, &mut server).await
                            }).await;
                        });
                    }
                    _ = clients.join_next(), if !clients.is_empty() => {}
                }
            }
        })))
    }
}

impl Drop for Forward {
    fn drop(&mut self) {
        self.0.abort();
    }
}
