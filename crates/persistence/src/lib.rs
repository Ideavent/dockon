use anyhow::Result;
use dockon_core::{Manifest, Operation};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row, SqlitePool,
};
use std::path::Path;

#[derive(Clone)]
pub struct Store(pub SqlitePool);
impl Store {
    pub async fn open(path: &Path) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true)
                    .foreign_keys(true),
            )
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        sqlx::query("UPDATE operations SET status='interrupted', error='Daemon stopped during operation' WHERE status='running'").execute(&pool).await?;
        Ok(Self(pool))
    }
    pub async fn list(&self) -> Result<Vec<Manifest>> {
        sqlx::query("SELECT manifest FROM projects ORDER BY id")
            .fetch_all(&self.0)
            .await?
            .into_iter()
            .map(|row| Ok(serde_json::from_str(row.try_get("manifest")?)?))
            .collect()
    }
    pub async fn register(&self, manifest: &Manifest) -> Result<()> {
        sqlx::query("INSERT INTO projects(id,manifest) VALUES (?,?)")
            .bind(&manifest.id)
            .bind(serde_json::to_string(manifest)?)
            .execute(&self.0)
            .await?;
        Ok(())
    }
    pub async fn begin(&self, id: &str, project: &str, action: &str) -> Result<()> {
        sqlx::query("INSERT INTO operations(id,project_id,action,status) VALUES (?,?,?,'running')")
            .bind(id)
            .bind(project)
            .bind(action)
            .execute(&self.0)
            .await?;
        Ok(())
    }
    pub async fn finish(&self, id: &str, error: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE operations SET status=?,error=? WHERE id=?")
            .bind(if error.is_some() {
                "failed"
            } else {
                "completed"
            })
            .bind(error)
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(())
    }
    pub async fn operation(&self, id: &str) -> Result<Operation> {
        let row =
            sqlx::query("SELECT id, project_id, action, status, error FROM operations WHERE id=?")
                .bind(id)
                .fetch_one(&self.0)
                .await?;
        Ok(Operation {
            id: row.try_get("id")?,
            project_id: row.try_get("project_id")?,
            action: row.try_get("action")?,
            status: row.try_get("status")?,
            error: row.try_get("error")?,
        })
    }
    pub async fn desired(&self, id: &str, running: bool) -> Result<()> {
        sqlx::query("UPDATE projects SET desired_state=? WHERE id=?")
            .bind(if running { "RUNNING" } else { "STOPPED" })
            .bind(id)
            .execute(&self.0)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn migrations_reopen_and_interrupted_operations() {
        let path = std::env::temp_dir().join(format!("dockon-db-{}.sqlite", uuid::Uuid::new_v4()));
        let store = Store::open(&path).await.unwrap();
        store.begin("op", "project", "start").await.unwrap();
        store.0.close().await;
        let reopened = Store::open(&path).await.unwrap();
        let operation = reopened.operation("op").await.unwrap();
        assert_eq!(operation.status, "interrupted");
        assert!(operation.error.unwrap().contains("Daemon stopped"));
        reopened.0.close().await;
        std::fs::remove_file(path).unwrap();
    }
}
