# Compatibility

| Capability | Windows x64 | macOS | Linux |
|---|---|---|---|
| Rust daemon / desktop | Verification in progress | CI configured, not run | CI configured, not run |
| IPC | Named Pipe | Unix socket, unverified | Unix socket, unverified |
| Native lifecycle | Job Object | Process group, unverified | Process group, unverified |
| Containers, DB, mail, AI | Not implemented | Not implemented | Not implemented |
| Signed installer/updater | Not implemented | Not implemented | Not implemented |

CI configuration alone is not evidence that remote platform builds passed.
