# Delivery status

Work spans the remaining runtime and database acceptance checks and the first Sprint 05 diagnostics.

| Sprint | Status |
|---|---|
| 01 foundation and lifecycle | Complete: Windows host/native UI checks and Windows/macOS/Linux CI passed |
| 02 hybrid provider and isolation | Complete: Windows/macOS/Linux CI and Linux Docker integration passed at 4138370 |
| 03 runtimes and web | Apache/Nginx switching for static and FastCGI projects verified on Windows. .htaccess execution remains pending. |
| 04 databases, cache, mail | Redis, Mailpit, PostgreSQL, MySQL, MariaDB and MongoDB Docker presets passed local persistence checks. |
| 05 updates, backup, diagnostics | Local aggregate diagnostics report and JSON export. Runtime updates and advisory lookup pending. |
| 06 cloud AI | Pending |

Windows x64 has an unsigned development installer with `C:\dockon` as its destination. Signed releases remain pending.

Local checks passed: workspace build; Rust tests including malformed/oversized IPC frames, coalesced stream frames and SQLite recovery; warning-free Clippy; frontend production build; native lifecycle smoke; event subscription; browser localization/offline checks; real Windows Tauri/WebView2 import/start/HTTP/stop flow. Foundation CI passed on all three OS targets at ee1ccde. The intermediate 7003277 workflow failed because a cache action could not be resolved; ee1ccde fixed the action and passed.

Sprint 02 uses immutable image digests, installation/project/instance ownership labels, isolated per-project networks and per-instance persistent volumes. Tested mixed native app + Redis container, two-project data/port/network isolation, preserved Redis data after restart, container HTTP app, DAG cycle rejection, failed-dependency rollback, and daemon crash reconciliation. The Unix ownership runner passed macOS/Linux CI. Podman execution remains unverified.

Sprint 03 adds side-by-side Node.js, Windows PHP NTS, .NET SDK, Go and Caddy installations from official publisher metadata and SHA-256/SHA-512 manifests. Archive extraction rejects escaping paths and links, bounds expanded sizes and stages before version checking and registration. Publisher signature verification is not implemented; the UI says so. Catalogs and verified archives are cached; no automatic upgrades or global PATH changes occur.

Project creation/import explicitly selects a registered installation. Startup checks executable integrity and rejects a mismatched executable. Source creation refuses existing target directories. Detection/import preserve existing source; no dependency installer is launched. Node/PHP/.NET/Go profiles and optional system Python/Rust profiles have separate launch adapters. Python and Rust managed downloads remain unsupported. Windows/macOS/Linux checks passed private Python venv creation/reuse and interpreter mismatch rejection, concrete Rust toolchain execution and private build output at a0e77d7. PHP uses per-project INI; runtime caches and mutable CLI state use private project data. Initial launch profiles are intentionally limited to detected supported entry points; a custom manifest remains available. A system executable can also be selected by its absolute path without changing global PATH. Native Windows UI checks covered both invalid and valid explicit executable selection.

The user supplies their own AI provider. Vendor, model and protocol stay unconfigured until its API contract is provided; see ADR 0003.

Routing uses editable full domains with `.test` defaults, private Caddy CA storage, loopback listeners and separate hosts/trust approvals. Backend checks passed HTTP/CA-verified HTTPS, independent stopped-route 503 updates, stale/replayed/forged approval rejection, failed-restart rollback, saved settings without autostart and unchanged real hosts on Windows/macOS/Linux CI at ae8b0a1. Native Windows UI checks passed editable `.test` routing, configuration preview/apply and separate hosts preview. Pure fixture tests passed scoped hosts writes, backup, preservation and stale-file rejection. Windows native certificate add/find/remove tests passed in an isolated memory store. Live elevation and real trust mutations remain unverified; macOS/Linux automated trust is unsupported.

Current implementation is on `codex/sprint03-runtimes`. The completed Sprint 02 checkpoint is on `main`. Windows checks passed managed Apache and Nginx installations, explicit executable registration, native static sites, live static-site switching, the Apache Docker sample and PHP/FastCGI import. See [web-server-boundary.md](web-server-boundary.md).

Stopped projects can add Redis, Mailpit, PostgreSQL, MySQL, MariaDB or MongoDB with Docker. Local checks verified Redis and Mailpit data isolation, SMTP delivery and inbox persistence, and authenticated database writes and reads after restart for all four database engines. Each managed database password lives in the daemon's private data directory and is returned on an explicit connection details request. PostgreSQL backups up to 8 MiB can be restored to a new database after a SHA-256 integrity check. Native database providers and backup and restore for the other engines remain pending.
