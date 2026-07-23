# AI Handoff

## Current goal

Implement a safe Windows/macOS Cockpit CLI MVP that synchronizes encrypted Codex rollout sessions through a Google Drive for desktop directory without synchronizing authentication, configuration, logs, or live databases.

## Completed work

- Created the user fork `lee3423434234-max/cockpit-tools` and a scheduled/manual upstream synchronization workflow.
- Created implementation branch `codex/google-drive-sync` from the fork's current `main`.
- Added `cockpit-core::modules::codex_drive_sync` with:
  - recursive rollout discovery under `sessions` and `archived_sessions`;
  - canonical SHA-256 hashing that excludes destination-specific `cwd` and `model_provider`;
  - per-session AES-256-GCM encrypted immutable objects with PBKDF2-HMAC-SHA256 keys;
  - per-device Drive heads, partial-file rejection, and local-only state;
  - stopped-Codex enforcement, upload-only and dry-run modes;
  - new-session import, idempotence, strict-prefix fast-forward, conflict quarantine, explicit conflict resolution, backup, index upsert, and pending rebuild recovery;
  - Windows atomic replacement through `MoveFileExW` and POSIX atomic rename;
  - source/destination cwd mapping and provider rewrite without hash feedback loops.
- Added `cockpit-core::modules::codex_app_server` to invoke the official Codex app-server metadata rebuild on Windows/macOS.
- Added CLI commands:
  - `cockpit-cli codex sessions status`
  - `cockpit-cli codex sessions sync-once`
  - `cockpit-cli codex sessions daemon`
  - `cockpit-cli codex sessions resolve-conflict`
- Added unit/integration tests for encryption/tamper rejection, canonical hashing, import idempotence, strict-prefix fast-forward, divergence, metadata mapping, partial files, running-process rejection, upload-only mode, explicit conflict resolution, and local-state isolation.
- Added Windows/macOS GitHub Actions CI and `docs/CODEX_DRIVE_SYNC.md`.
- Installed Rustup stable locally.

## In-progress work

- Visual C++ Build Tools are being installed because the existing Build Tools instance lacked the MSVC linker.
- Rust formatting parses successfully and `cargo metadata` succeeds. Full compile/test validation is waiting for the linker installation to finish.

## Next steps

1. Run `cargo test -p cockpit-core codex_drive_sync --no-fail-fast`.
2. Run `cargo check -p cockpit-cli` and validate CLI help/status behavior.
3. Fix all compiler, Clippy, and test failures; run focused failure-injection tests again.
4. Review the diff for secrets, unsafe path handling, accidental live database access, and unrelated changes.
5. Commit and push `codex/google-drive-sync` to the user fork.
6. Let Windows/macOS CI complete before enabling real bidirectional imports.
7. Deploy on the first computer with `--upload-only --dry-run`, then `--upload-only`; configure the second device only after verifying encrypted object/head counts.

## Risks or blockers

- Do not run real imports while any Codex process is active.
- The passphrase must be shared out-of-band and must never be committed, logged, or written to Google Drive.
- The current encrypted object implementation processes one rollout in memory. The largest observed local rollout is about 79 MB; streaming encryption/compression is a future optimization.
- Google Drive File Provider can expose a head before its object is hydrated; missing objects and `.partial` files are deliberately ignored until a later poll.
- A metadata rebuild failure leaves local state marked pending and requires a later successful app-server run.
- Automatic divergent branch merging is intentionally unsupported. A selected conflict object can overwrite only through the explicit resolve command after backup.
- macOS behavior still requires hosted/physical Mac validation, especially File Provider hydration, permissions, app-server discovery, and atomic rename behavior.
- Cockpit Tools is CC BY-NC-SA 4.0; commercial/internal business use requires separate authorization.
