# Compatibility

| Capability | Windows x64 | macOS | Linux |
|---|---|---|---|
| Rust daemon / desktop | Local build and native UI E2E pass | Baseline CI passed | CI running |
| IPC | Named Pipe, authenticated and streamed | Unix socket baseline CI passed | Unix socket CI running |
| Native lifecycle | Job Object, lifecycle/tree tests pass | Process group baseline CI passed | Process group CI running |
| Containers, DB, mail, AI | Not implemented | Not implemented | Not implemented |
| Signed installer/updater | Not implemented | Not implemented | Not implemented |

CI configuration alone is not evidence that remote platform builds passed.
