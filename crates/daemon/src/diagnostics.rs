use crate::DaemonState;
use anyhow::Result;
use dockon_core::State;
use serde_json::{json, Value};

pub async fn report(state: &DaemonState) -> Result<Value> {
    let (projects, runtimes, router) = tokio::try_join!(
        state.projects.list(),
        state.runtimes.installed(),
        state.router.status()
    )?;
    let states = [
        State::Stopped,
        State::Starting,
        State::Running,
        State::Degraded,
        State::Stopping,
        State::Failed,
    ];
    let mut project_counts = [0; 6];
    let mut service_counts = [0; 6];
    for (index, status) in states.iter().enumerate() {
        project_counts[index] = projects.iter().filter(|p| p.state == *status).count();
        service_counts[index] = projects
            .iter()
            .flat_map(|p| &p.services)
            .filter(|s| s.state == *status)
            .count();
    }
    let runtimes_with_archive_hash = runtimes.iter().filter(|r| r.archive_hash.is_some()).count();
    let runtimes_with_verified_signature = runtimes.iter().filter(|r| r.signature_verified).count();
    Ok(json!({
        "schema_version": 1,
        "dockon_version": env!("CARGO_PKG_VERSION"),
        "platform": { "os": std::env::consts::OS, "arch": std::env::consts::ARCH },
        "project_counts": counts(states, project_counts),
        "service_counts": counts(states, service_counts),
        "runtimes": {
            "installed": runtimes.len(),
            "archive_hash_recorded": runtimes_with_archive_hash,
            "publisher_signature_verified": runtimes_with_verified_signature,
        },
        "router": { "running": router.running, "has_error": router.error.is_some() }
    }))
}

fn counts(states: [State; 6], counts: [usize; 6]) -> Value {
    Value::Object(
        states
            .into_iter()
            .zip(counts)
            .map(|(state, count)| {
                let label = json!(state).as_str().expect("state label").to_owned();
                (label, json!(count))
            })
            .collect(),
    )
}
