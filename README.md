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

Paste the returned absolute path into Import manifest. Alternatively, select an installed runtime on the Runtimes page and use New / import to create a project or detect an existing source folder. Projects start only when requested. Manifest imports do not execute code; explicit system runtime selection checks the selected executable's version. UI and daemon run as ordinary user. Closing the UI leaves the daemon and running projects intact. Run `target\debug\dockonctl shutdown` to stop the daemon and its workloads.

## Checks

```powershell
pnpm build
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
node scripts/smoke.mjs
node scripts/hybrid-smoke.mjs
node scripts/project-smoke.mjs
node scripts/runtime-smoke.mjs
node scripts/router-smoke.mjs
node scripts/ui-smoke.mjs
node scripts/desktop-smoke.mjs
```

The smoke test uses an isolated temporary data directory. It verifies real IPC/auth, persistence, two-project isolation, HTTP readiness, idempotence, failure and cleanup.

The browser/native UI checks expect the Vite dev server to be running (`pnpm dev`). `desktop-smoke.mjs` is a Windows WebView2 test; its temporary debugging port is loopback-only and enabled exclusively on the isolated test process, never in normal app configuration.

`hybrid-smoke.mjs` requires a running Docker Linux engine. It downloads only digest-pinned sample images as needed, creates uniquely labeled disposable test resources, verifies two-project Redis data isolation and recovery, then removes only those verified test resources. It does not touch existing Docker projects or images. Generic container launches are implemented; managed database catalogs and backup tooling are later sprints.

`runtime-smoke.mjs` downloads the current official Node LTS into an isolated test data directory, verifies its digest/version, runs a project and checks offline catalog recovery. Set `DOCKON_EXTENDED_RUNTIMES=1` to also download and verify supported PHP/.NET/Go/Caddy artifacts and launch project profiles. These are network integration checks, not publisher signature verification.

Windows installed application target: C:\dockon. No production installer is available yet. Windows/macOS/Linux foundation and hybrid checks passed in the configured GitHub repository; new runtime work is still being verified. User projects and backups must survive uninstallation.

The Local domains page defaults project names to `.test` and accepts edited full hostnames. Select a Caddy installation, projects and unprivileged ports, then preview and apply. The router does not start project processes. Hosts records and user CA trust have separate permission previews. Tests never apply changes to real system hosts or trust stores; live permission integration remains unverified. Windows trust targets the current user's ROOT store; macOS/Linux automatic trust is not yet supported.
