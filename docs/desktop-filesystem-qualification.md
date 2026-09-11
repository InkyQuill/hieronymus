# Desktop filesystem qualification

Task 8 supplies native filesystem implementations and native tests. Cross-target
source checks do not establish native runtime or installed-artifact acceptance.
Run this checklist on disposable local storage as the normal desktop user after
the platform dependency graph is available. Do not point tests at a book project,
an installed application directory, or a running daemon's data root.

Record commit, OS/version, architecture, filesystem, Rust version, exact commands,
exit codes, and complete logs. Windows x64 and Apple Silicon macOS are the first
available native hosts; Intel macOS still needs its own qualification.

```text
cargo check -p hiero --all-targets --all-features --locked
cargo test -p hiero -p hieronymus --all-features --locked --test platform_filesystem --test export_safety --test dream_locks_port --test upgrade_port
cargo test -p hiero -p hieronymus --all-features --locked --lib private_file
cargo test -p hiero -p hieronymus --all-features --locked --lib atomic::
cargo test -p hiero --all-features --locked --lib platform::export
cargo test -p hiero --all-features --locked --lib publication_tests
```

The Windows suite uses native `icacls.exe` to add an Everyone-read ACE to a
**disposable credential**, and `cmd.exe /C mklink /J` to create a **disposable
junction**. Fixture failures are failures, not skipped acceptance. Tests cover
creation-time protected owner DACL, inherited/permissive ACL refusal, hardlink
aliases, exclusive publication, immutable launcher selection, directory
replacement, and junction refusal. Unix tests retain descriptor-relative export
and symbolic/hardlink regressions. The dream suite creates one child test process
that exits without dropping its guard, then proves a successor can own the lock.

Windows directory durability requires particular attention. Ordinary atomic and
private-file publication flush contents, perform write-through `MoveFileExW`, and
request a directory flush using a write-capable backup-semantics handle. A native
open/flush failure propagates: there is no success fallback or administrator
volume-flush requirement. Such a failure can occur **after the new name has been
published**; inspect state before retrying. Do not report an error as proof that
the destination stayed unchanged. Upgrade journal/root ownership remains held
across these operations.

Windows export walks components with `NtCreateFile` relative to retained handles,
rejects reparse points, exclusively creates an owner-protected temporary file,
flushes it, and renames the opened file with `FILE_RENAME_INFO.RootDirectory`.
No-clobber uses `ReplaceIfExists = false`. It then flushes the file again. A
post-rename flush error preserves the published output; cleanup may delete only
the still-unpublished temporary handle. Native power-loss durability remains a
qualification requirement; output-file flush is not represented as POSIX
directory-fsync equivalence.

## Windows selected-version and launcher interface

Build target `hiero-launcher` produces `hiero-launcher.exe`. Each immutable version
payload contains `hiero.exe` and `hiero-launcher.exe`. Initial selection creates
`bin/hiero.exe`, `bin/hieronymus.exe`, `bin/hieronymus-mcp.exe`, and
`bin/hieronymus-agent-hook.exe` from the launcher without overwriting existing
files. Every endpoint reads the single ordinary, non-secret authority record:

```json
{"version":"1.2.3","launcher_sha256":"64 lowercase hex characters"}
```

The record is `<app>/selected-version.json`. Versions must be normal single
ASCII directory names; path separators, streams, dot components and trailing dots
are refused. The launcher verifies every stable endpoint against the recorded
hash, selects `versions/<version>/hiero.exe`, preserves arguments, maps historical
MCP/hook names to their subcommands, inherits standard streams, and returns the
child exit code. Its Windows subsystem and child `CREATE_NO_WINDOW` flag avoid
opening a console window. Native invocation, redirected stdio, and exit-code
behavior must be tested with the real packaged executable before acceptance.

The application root is per-user trusted storage. A corrupt selection or foreign
launcher causes an error. Switching versions preserves the original launchers
and atomically changes only the record. The JSON record is not an authenticity
signature against the same account. Later update/helper coordination owns
launcher replacement, crash repair of partially created initial endpoints,
running-executable cleanup, and complete installed-artifact rollback/uninstall.
Do not infer those capabilities from filesystem tests. Runtime acquisition and
per-target ONNX hashes likewise have separate qualification gates.

Native API references: [FILE_RENAME_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info)
and [FlushFileBuffers](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers).
The latter documents the write-handle requirement, not a portable guarantee for
directory flushes.
