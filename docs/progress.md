# Delivery status

Work is in Sprint 03. Later sprints must not be called complete before their implementations and acceptance checks pass.

| Sprint | Status |
|---|---|
| 01 foundation and lifecycle | Complete: Windows host/native UI checks and Windows/macOS/Linux CI passed |
| 02 hybrid provider and isolation | Complete: Windows/macOS/Linux CI and Linux Docker integration passed at 4138370 |
| 03 runtimes and web | In progress: publisher catalogs, managed installers and explicit runtime/project profiles implemented; router, HTTPS and server switching remain |
| 04 databases, cache, mail | Pending |
| 05 updates, backup, diagnostics | Pending |
| 06 cloud AI | Pending |

Unimplemented capabilities return not_implemented from health. No production installer or signed release exists yet.

Local checks passed: workspace build; Rust tests including malformed/oversized IPC frames, coalesced stream frames and SQLite recovery; warning-free Clippy; frontend production build; native lifecycle smoke; event subscription; browser localization/offline checks; real Windows Tauri/WebView2 import/start/HTTP/stop flow. Foundation CI passed on all three OS targets at ee1ccde. The intermediate 7003277 workflow failed because a cache action could not be resolved; ee1ccde fixed the action and passed.

Sprint 02 uses immutable image digests, installation/project/instance ownership labels, isolated per-project networks and per-instance persistent volumes. Tested mixed native app + Redis container, two-project data/port/network isolation, preserved Redis data after restart, container HTTP app, DAG cycle rejection, failed-dependency rollback, and daemon crash reconciliation. The Unix ownership runner passed macOS/Linux CI. Podman execution remains unverified.

Sprint 03 adds side-by-side Node.js, Windows PHP NTS, .NET SDK, Go and Caddy installations from official publisher metadata and SHA-256/SHA-512 manifests. Archive extraction rejects escaping paths and links, bounds expanded sizes and stages before version checking and registration. Publisher signature verification is not implemented; the UI says so. Catalogs and verified archives are cached; no automatic upgrades or global PATH changes occur.

Project creation/import explicitly selects a registered installation. Startup checks executable integrity and rejects a mismatched executable. Source creation refuses existing target directories. Detection/import preserve existing source; no dependency installer is launched. Node/PHP/.NET/Go profiles and optional system Python/Rust profiles have separate launch adapters. Python and Rust managed downloads remain unsupported; Python venv and concrete Rust toolchain profiles need host verification. PHP uses per-project INI; runtime caches and mutable CLI state use private project data. Initial launch profiles are intentionally limited to detected supported entry points; a custom manifest remains available.

The user supplies their own AI provider. Vendor, model and protocol stay unconfigured until its API contract is provided; see ADR 0003.
