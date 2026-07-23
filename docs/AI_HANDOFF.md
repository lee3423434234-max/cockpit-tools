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
- Installed the Visual C++ toolchain and Windows SDK required by the Rust MSVC target.
- Fixed Windows process detection export, Rust error lifetimes, CLI `String` to `anyhow` conversion, and large enum layout warnings found during local compilation.
- Verified 11 focused Drive-sync tests, `cargo check -p cockpit-cli`, CLI help, and read-only status against isolated nonexistent test paths.
- Pushed implementation commits `068a84a`, `97142af`, and `8bbd5c0` to `fork/codex/google-drive-sync`.

## In-progress work

- GitHub Actions run `30035109511` is validating commit `8bbd5c0` on `windows-latest` and `macos-latest`.
- Real Google Drive session synchronization remains intentionally disabled until CI is green and the first-device upload-only rollout is explicitly started.

## Next steps

1. Wait for GitHub Actions run `30035109511` and fix any Windows/macOS failure before deployment.
2. Create a pull request from `codex/google-drive-sync` to the fork's `main` only after CI is green and the user wants integration.
3. On the first computer, close Codex and run `--upload-only --dry-run` against a private Drive directory with the passphrase supplied out-of-band.
4. Review counts and paths, then run `--upload-only`; do not enable imports yet.
5. Validate File Provider hydration and app-server discovery on a physical Mac before the first macOS import.
6. Configure the second device with explicit `--map-cwd` values, dry-run first, and enable bidirectional imports only after backup verification.

## Risks or blockers

- Do not run real imports while any Codex process is active.
- The passphrase must be shared out-of-band and must never be committed, logged, or written to Google Drive.
- The current encrypted object implementation processes one rollout in memory. The largest observed local rollout is about 79 MB; streaming encryption/compression is a future optimization.
- Google Drive File Provider can expose a head before its object is hydrated; missing objects and `.partial` files are deliberately ignored until a later poll.
- A metadata rebuild failure leaves local state marked pending and requires a later successful app-server run.
- Automatic divergent branch merging is intentionally unsupported. A selected conflict object can overwrite only through the explicit resolve command after backup.
- macOS behavior still requires hosted/physical Mac validation, especially File Provider hydration, permissions, app-server discovery, and atomic rename behavior.
- Cockpit Tools is CC BY-NC-SA 4.0; commercial/internal business use requires separate authorization.
- Workspace-wide Clippy with `-D warnings` is blocked by more than 180 pre-existing upstream warnings. The new Drive-sync files have no Clippy warnings; two remaining warnings in `cockpit-cli/src/main.rs` are pre-existing lines last changed by upstream commit `1e2b1a9`.
