# dockon

Cross-platform local development environment manager. Working foundation under development; see [progress](docs/progress.md) and [limitations](docs/adr/0001-foundation.md).

## Windows development

From C:\dockon_build in PowerShell:

```powershell
. .\scripts\Enter-Dev.ps1
pnpm install --frozen-lockfile
cargo build --workspace
pnpm desktop
```

Rust toolchain is installed locally in .tools on this development machine. On other machines install stable Rust, Node 24+, pnpm 11.25.0 and official Tauri 2 prerequisites. The application does not change global PATH.

Create/import a sample after building:

```powershell
.\scripts\New-SampleManifest.ps1
```

Paste the returned absolute path into Import project. Projects start only when requested. Imports do not execute code. UI and daemon run as ordinary user. Closing the UI leaves the daemon and running projects intact. Run `target\debug\dockonctl shutdown` to stop the daemon and its workloads.

## Checks

```powershell
pnpm build
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node scripts/smoke.mjs
```

The smoke test uses an isolated temporary data directory. It verifies real IPC/auth, persistence, two-project isolation, HTTP readiness, idempotence, failure and cleanup.

Windows installed application target: C:\dockon. No production installer is available in Sprint 01. macOS/Linux builds require their own host dependencies and will be verified through CI when a GitHub remote is supplied. User projects and backups must survive uninstallation.
