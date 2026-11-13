use anyhow::{bail, Context, Result};
use dockon_core::Command;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let command = match args.first().map(String::as_str) {
        Some("health") => Command::Health,
        Some("capabilities") => Command::Capabilities,
        Some("runtimes") => Command::RuntimeInstalled,
        Some("catalog") => Command::RuntimeCatalog {
            runtime: args.get(1).context("Runtime id required")?.clone(),
        },
        Some("install") => Command::RuntimeInstall {
            runtime: args.get(1).context("Runtime id required")?.clone(),
            version: args.get(2).context("Version required")?.clone(),
        },
        Some("discover") => Command::RuntimeDiscover,
        Some("list") => Command::List,
        Some("register") => Command::Register {
            manifest_path: args.get(1).context("Manifest path required")?.into(),
        },
        Some("start") => Command::Start {
            id: args.get(1).context("Project id required")?.clone(),
        },
        Some("stop") => Command::Stop {
            id: args.get(1).context("Project id required")?.clone(),
        },
        Some("events") => Command::Events { after: 0 },
        Some("operation") => Command::Operation {
            id: args.get(1).context("Operation id required")?.clone(),
        },
        Some("shutdown") => Command::Shutdown,
        _ => bail!(
            "Usage: dockonctl health|list|register <manifest>|start <id>|stop <id>|events|shutdown"
        ),
    };
    let response = dockon_ipc::call(command).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    if !response.ok {
        std::process::exit(1);
    }
    Ok(())
}
