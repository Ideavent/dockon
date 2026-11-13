# Delivery status

Work is in Sprint 01. Later sprints must not be called complete before their implementations and acceptance checks pass.

| Sprint | Status |
|---|---|
| 01 foundation and lifecycle | Complete: Windows host/native UI checks and Windows/macOS/Linux CI passed |
| 02 hybrid provider and isolation | Local Docker/Redis isolation, DAG rollback and recovery checks pass; cross-platform hardening in progress |
| 03 runtimes and web | Pending |
| 04 databases, cache, mail | Pending |
| 05 updates, backup, diagnostics | Pending |
| 06 cloud AI | Pending |

Unimplemented capabilities return not_implemented from health. No production installer or signed release exists yet.

Local checks passed: workspace build; Rust tests including malformed/oversized IPC frames, coalesced stream frames and SQLite recovery; warning-free Clippy; frontend production build; native lifecycle smoke; event subscription; browser localization/offline checks; real Windows Tauri/WebView2 import/start/HTTP/stop flow. Foundation CI passed on all three OS targets at ee1ccde. The intermediate 7003277 workflow failed because a cache action could not be resolved; ee1ccde fixed the action and passed.

Sprint 02 uses immutable image digests, installation/project/instance ownership labels, isolated per-project networks and per-instance persistent volumes. Tested mixed native app + Redis container, two-project data/port/network isolation, preserved Redis data after restart, container HTTP app, DAG cycle rejection, failed-dependency rollback, and daemon crash reconciliation. Podman execution and Unix runner changes still require host/CI verification.
