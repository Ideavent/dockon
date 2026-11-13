# Explicit runtime bindings and project profiles

Runtime installation is separate from project execution. A project records an installation ID and executable, and startup verifies the stored executable digest before preparing private mutable runtime state. Downloads are immutable side-by-side installations; installing another version does not switch a running project. No global PATH or language-manager configuration is edited.

Only explicit typed argument templates expand Dockon host, port and data paths. Existing string arguments retain literal semantics and never pass through a shell. New manifests add optional runtime bindings and startup timeouts without invalidating existing v1 manifests.

Publisher HTTPS metadata and archive checksums establish the implemented download boundary. They do not establish publisher signature verification; records and UI explicitly report that missing capability. Python/Rust managed distributions are not selected implicitly. System tools require user selection before version inspection; a concrete Rust Cargo toolchain is required to avoid automatic rustup installation. Python isolation uses a per-project venv tied to its base executable digest.

Creating a project requires a nonexistent target directory beneath an existing parent. Import detects supported launch profiles and does not rewrite source or install dependencies. A failed creation preserves any generated files for inspection instead of recursively deleting source. .NET templates pin the selected SDK; imports reject a conflicting global.json. Windows paths passed to external CLIs use ordinary canonical paths where supported because Node CLI cannot consistently consume verbatim Windows paths.
