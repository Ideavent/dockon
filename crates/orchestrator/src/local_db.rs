use anyhow::{bail, Context, Result};
use dockon_core::{HealthCheck, LaunchArg, Manifest, ServiceSpec};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Postgres,
    Mysql,
}
impl Kind {
    pub fn parse(id: &str) -> Result<Self> {
        match id {
            "postgres" => Ok(Self::Postgres),
            "mysql" => Ok(Self::Mysql),
            _ => bail!("Local database options are PostgreSQL and MySQL"),
        }
    }
    fn binary(self) -> &'static str {
        match self {
            Self::Postgres => "postgres.exe",
            Self::Mysql => "mysqld.exe",
        }
    }
    fn folder(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
        }
    }
    fn bin(self) -> Result<PathBuf> {
        let exe = std::env::current_exe()?;
        Ok(exe
            .parent()
            .context("Missing Dockon directory")?
            .join("bin")
            .join(self.folder())
            .join("bin"))
    }
    pub fn matches(self, path: &Path) -> bool {
        path.ends_with(
            Path::new("bin")
                .join(self.folder())
                .join("bin")
                .join(self.binary()),
        )
    }
    pub fn service(self) -> Result<ServiceSpec> {
        let executable = self.bin()?.join(self.binary());
        if !executable.is_file() {
            bail!(
                "{} is included in the Dockon Windows package",
                self.folder()
            );
        }
        Ok(serde_json::from_value(serde_json::json!({
            "id": self.folder(), "executable": executable, "health": HealthCheck::Tcp, "startup_timeout_seconds":120
        }))?)
    }
    pub fn password_file(self, data: &Path) -> PathBuf {
        data.join(if self == Self::Postgres {
            "postgres-password"
        } else {
            "database-password"
        })
    }
    pub fn user(self) -> &'static str {
        if self == Self::Postgres {
            "postgres"
        } else {
            "root"
        }
    }
}

pub struct Database {
    pub kind: Kind,
    bin: PathBuf,
    data: PathBuf,
    password: String,
    port: u16,
}
impl Database {
    pub async fn prepare(
        kind: Kind,
        manifest: &mut Manifest,
        data: &Path,
        port: u16,
    ) -> Result<Self> {
        let bin = kind.bin()?;
        let password_file = kind.password_file(data);
        if !password_file.is_file() {
            let password = format!("Aa9-{}", uuid::Uuid::new_v4().simple());
            use std::io::Write;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&password_file)?
                .write_all(password.as_bytes())?;
        }
        let password = fs::read_to_string(&password_file)?;
        if password.is_empty()
            || !password
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            bail!("Invalid managed database credential");
        }
        let db = Self {
            kind,
            bin,
            data: data.into(),
            password,
            port,
        };
        let files = db.cluster();
        let initial = !files.is_dir();
        if initial {
            let staging = data.join(format!("initialize-{}", uuid::Uuid::new_v4()));
            match kind {
                Kind::Postgres => {
                    db.run(
                        "initdb.exe",
                        &[
                            "-D".into(),
                            staging.display().to_string(),
                            "-U".into(),
                            "postgres".into(),
                            "--auth=scram-sha-256".into(),
                            "--encoding=UTF8".into(),
                            "--locale=C".into(),
                            format!("--pwfile={}", password_file.display()),
                        ],
                    )
                    .await?;
                }
                Kind::Mysql => {
                    db.run(
                        "mysqld.exe",
                        &[
                            "--no-defaults".into(),
                            "--initialize-insecure".into(),
                            format!("--datadir={}", staging.display()),
                        ],
                    )
                    .await?;
                    fs::write(data.join("first-start.sql"), format!(
                        "ALTER USER 'root'@'localhost' IDENTIFIED BY '{}';\nCREATE DATABASE IF NOT EXISTS dockon;\n", db.password
                    ))?;
                }
            }
            fs::rename(staging, &files)?;
        }
        manifest.executable = db.bin.join(kind.binary());
        manifest.args = match kind {
            Kind::Postgres => vec![
                "-D".into(),
                files.display().to_string(),
                "-h".into(),
                "127.0.0.1".into(),
                "-p".into(),
                port.to_string(),
            ],
            Kind::Mysql => {
                let mut args = vec![
                    "--no-defaults".into(),
                    "--mysqlx=OFF".into(),
                    "--bind-address=127.0.0.1".into(),
                    format!("--port={port}"),
                    format!("--datadir={}", files.display()),
                    "--console".into(),
                ];
                let init = data.join("first-start.sql");
                if init.is_file() {
                    args.push(format!("--init-file={}", init.display()));
                }
                args
            }
        }
        .into_iter()
        .map(LaunchArg::from)
        .collect();
        Ok(db)
    }

    fn cluster(&self) -> PathBuf {
        self.data.join("native-data")
    }

    fn pg_args(&self, database: &str) -> Vec<String> {
        vec![
            "-h".into(),
            "127.0.0.1".into(),
            "-p".into(),
            self.port.to_string(),
            "-U".into(),
            "postgres".into(),
            "-d".into(),
            database.into(),
        ]
    }
    async fn sql(&self, database: &str, statement: &str) -> Result<Vec<u8>> {
        let mut args = self.pg_args(database);
        args.extend([
            "-X".into(),
            "-At".into(),
            "-v".into(),
            "ON_ERROR_STOP=1".into(),
            "-c".into(),
            statement.into(),
        ]);
        self.run("psql.exe", &args).await
    }
    pub async fn initialize(&self) -> Result<()> {
        match self.kind {
            Kind::Postgres => {
                let exists = self
                    .sql(
                        "postgres",
                        "SELECT 1 FROM pg_database WHERE datname = 'dockon'",
                    )
                    .await?;
                if String::from_utf8_lossy(&exists).trim() != "1" {
                    self.sql("postgres", "CREATE DATABASE dockon").await?;
                }
                let dump = self.data.join("pending-dockon.dump");
                if dump.is_file() {
                    self.restore_archive("dockon", &dump).await?;
                    fs::rename(&dump, self.data.join("imported-dockon.dump"))?;
                }
                let dump = self.data.join("pending-postgres.dump");
                if dump.is_file() {
                    self.restore_archive("postgres", &dump).await?;
                    fs::rename(&dump, self.data.join("imported-postgres.dump"))?;
                }
            }
            Kind::Mysql => {
                self.mysql("mysql.exe", "--execute=SELECT 1").await?;
                let init = self.data.join("first-start.sql");
                if init.is_file() {
                    fs::remove_file(init)?;
                }
            }
        }
        Ok(())
    }
    pub async fn ready(&self) -> bool {
        if self.kind == Kind::Postgres {
            return self
                .run("pg_isready.exe", &self.pg_args("postgres"))
                .await
                .is_ok();
        }
        self.mysql("mysql.exe", "--execute=SELECT 1").await.is_ok()
    }
    async fn restore_archive(&self, database: &str, path: &Path) -> Result<()> {
        let mut args = self.pg_args(database);
        args.extend([
            "--single-transaction".into(),
            "--exit-on-error".into(),
            path.display().to_string(),
        ]);
        self.run("pg_restore.exe", &args).await?;
        Ok(())
    }
    pub async fn dump(&self) -> Result<Vec<u8>> {
        if self.kind == Kind::Postgres {
            self.run("pg_dump.exe", &self.pg_args("dockon")).await
        } else {
            let mut args = self.mysql_args();
            args.extend(
                [
                    "--single-transaction",
                    "--no-tablespaces",
                    "--set-gtid-purged=OFF",
                    "dockon",
                ]
                .map(str::to_owned),
            );
            self.run("mysqldump.exe", &args).await
        }
    }
    pub async fn restore_copy(&self, sql: &[u8], database: &str) -> Result<()> {
        if !database
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            bail!("Invalid backup destination");
        }
        if self.kind == Kind::Mysql {
            self.mysql(
                "mysql.exe",
                &format!("--execute=CREATE DATABASE {database}"),
            )
            .await?;
            let mut args = self.mysql_args();
            args.push(database.into());
            if let Err(error) = self.run_input("mysql.exe", &args, Some(sql)).await {
                let _ = self
                    .mysql("mysql.exe", &format!("--execute=DROP DATABASE {database}"))
                    .await;
                return Err(error);
            }
            return Ok(());
        }
        self.sql("postgres", &format!("CREATE DATABASE {database}"))
            .await?;
        let path = self
            .data
            .join(format!("restore-{}.sql", uuid::Uuid::new_v4()));
        fs::write(&path, sql)?;
        let mut args = self.pg_args(database);
        args.extend([
            "-X".into(),
            "--single-transaction".into(),
            "-v".into(),
            "ON_ERROR_STOP=1".into(),
            "-f".into(),
            path.display().to_string(),
        ]);
        let result = self.run("psql.exe", &args).await;
        let _ = fs::remove_file(path);
        if let Err(error) = result {
            let _ = self
                .sql("postgres", &format!("DROP DATABASE {database}"))
                .await;
            return Err(error);
        }
        Ok(())
    }
    fn mysql_args(&self) -> Vec<String> {
        vec![
            "--no-defaults".into(),
            "--protocol=TCP".into(),
            "--host=127.0.0.1".into(),
            format!("--port={}", self.port),
            "--user=root".into(),
        ]
    }
    async fn mysql(&self, executable: &str, arg: &str) -> Result<Vec<u8>> {
        let mut args = self.mysql_args();
        args.push(arg.into());
        self.run(executable, &args).await
    }
    pub async fn stop(&self) -> Result<()> {
        if self.kind == Kind::Postgres {
            self.run(
                "pg_ctl.exe",
                &[
                    "-D".into(),
                    self.cluster().display().to_string(),
                    "-m".into(),
                    "fast".into(),
                    "-w".into(),
                    "-t".into(),
                    "10".into(),
                    "stop".into(),
                ],
            )
            .await?;
        } else {
            self.mysql("mysqladmin.exe", "shutdown").await?;
        }
        Ok(())
    }
    async fn run(&self, name: &str, args: &[String]) -> Result<Vec<u8>> {
        self.run_input(name, args, None).await
    }
    async fn run_input(
        &self,
        name: &str,
        args: &[String],
        input: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        let mut command = tokio::process::Command::new(self.bin.join(name));
        command
            .args(args)
            .current_dir(&self.data)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .env("PGPASSWORD", &self.password)
            .env("PGCONNECT_TIMEOUT", "2")
            .env("MYSQL_PWD", &self.password);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let output = tokio::time::timeout(Duration::from_secs(105), async {
            use tokio::io::AsyncWriteExt;
            let mut child = command.spawn()?;
            let stdin = child.stdin.take();
            let (_, output) = tokio::try_join!(
                async {
                    if let (Some(mut pipe), Some(input)) = (stdin, input) {
                        pipe.write_all(input).await?;
                    }
                    Ok::<_, std::io::Error>(())
                },
                child.wait_with_output()
            )?;
            Ok::<_, std::io::Error>(output)
        })
        .await
        .with_context(|| format!("{name} timed out"))??;
        if !output.status.success() {
            bail!(
                "{name} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(output.stdout)
    }
}
