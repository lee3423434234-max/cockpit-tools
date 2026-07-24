# AI Handoff

## Current goal

Deliver a safe Cockpit GUI and CLI that synchronize encrypted Codex rollout sessions through a Google Drive for desktop directory, shipping only for Windows x64 and macOS Apple Silicon (ARM64), without synchronizing authentication, configuration, logs, or live databases.

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
- GitHub Actions run `30035109511` passed on both `windows-latest` and `macos-latest` for code commit `8bbd5c0`.
- Opened fork pull request `#1` from `codex/google-drive-sync` into `main`: `https://github.com/lee3423434234-max/cockpit-tools/pull/1`.
- Fixed the CodeQL hard-coded-salt false positive by generating the PBKDF2 salt and AES-GCM nonce directly with `OsRng`; commit `1477e4a` passed Rust/JavaScript CodeQL and code-scanning checks.
- Confirmed all 14 pull-request checks passed for commit `1477e4a`, including Windows, macOS ARM64/x64/universal, Ubuntu, Drive-sync CI, and CodeQL.
- Restricted the fork's pull-request Build Matrix and release artifacts to Windows x64 (`x86_64-pc-windows-msvc`) and macOS Apple Silicon (`aarch64-apple-darwin`).
- Updated the updater manifest, Homebrew cask, manual release helper, and release documentation for the two supported architectures; macOS Intel/universal and Linux release jobs were removed.
- Added a release-manifest regression test and locally passed 14 Node release tests, YAML parsing, 11 Drive-sync tests, 2 app-server path tests, and `cargo check -p cockpit-cli`.
- Pushed architecture-policy commit `e199061`; the updated pull request passed all 8 checks, including the reduced `windows-x86_64` and `macos-aarch64` Build Matrix jobs.
- Merged pull request `#1` into the fork's `main` with merge commit `48b80898172547a004a259f6028ac12c86d3fec5`.
- Verified the fork's `main` contains the encrypted Drive sync feature, the two-architecture Build Matrix, and the scheduled/manual `.github/workflows/sync-upstream.yml` workflow.
- Implemented branch `codex/google-drive-sync-gui` with a lazy-loaded Settings → Data GUI, memory-only passphrase handling, Drive folder selection, CWD/provider mapping, read-only status, upload-only and bidirectional dry-runs/runs, conflict listing, and explicit conflict resolution.
- Added four Tauri commands backed directly by `cockpit-core` and moved blocking scans/sync work off the UI thread.
- Verified TypeScript typecheck, locale key consistency, production Vite build, two focused GUI bridge tests, and `cargo check -p cockpit-tools` on Windows x64. No real Drive sync or conflict resolution was executed.
- Pull request `#2` passed all 8 checks and merged into the fork's `main` as `6e2a9e7a482e39f5e021b5305a1555c55610e317`.
- Added a safe first-release bootstrap manifest, redirected updater/release-note URLs to the fork, generated a fork-specific Tauri updater key pair, configured its public key, stored the encrypted private key/password as repository secrets, and removed the temporary plaintext password.
- Pull request `#3` passed all 6 applicable checks (Windows x64, macOS ARM64, Preflight, Rust/JavaScript CodeQL, and code scanning) and merged into `main` as `a87e819318ea1655b81508b850bf104ad17ce364`.
- Published formal GitHub Release `v1.3.14`: `https://github.com/lee3423434234-max/cockpit-tools/releases/tag/v1.3.14`.
- Release workflow run `30107660341` succeeded on attempt 2 after the generated Homebrew Cask update was applied directly to `main` as `72f4f40`; repository-wide permission for Actions-created pull requests remained disabled.
- Verified 12 public release assets, HTTP 200 downloads for Windows EXE/MSI and macOS ARM64 DMG, signed updater archives, target manifests, the complete five-target legacy `latest.json`, and `SHA256SUMS.txt`.

## In-progress work

- The formal `v1.3.14` release is complete. Rendered GUI QA, physical Apple Silicon validation, and the first real Google Drive upload/import remain pending.

## Next steps

1. Perform rendered desktop/mobile GUI QA when an in-app Browser backend is available.
2. Validate Google Drive File Provider hydration and the full GUI flow on a physical Apple Silicon Mac before any macOS import.
3. Keep the first real deployment upload-only until encrypted objects and heads are inspected.

## Risks or blockers

- Do not run real imports while any Codex process is active.
- The passphrase must be shared out-of-band and must never be committed, logged, or written to Google Drive.
- The current encrypted object implementation processes one rollout in memory. The largest observed local rollout is about 79 MB; streaming encryption/compression is a future optimization.
- Google Drive File Provider can expose a head before its object is hydrated; missing objects and `.partial` files are deliberately ignored until a later poll.
- A metadata rebuild failure leaves local state marked pending and requires a later successful app-server run.
- Automatic divergent branch merging is intentionally unsupported. A selected conflict object can overwrite only through the explicit resolve command after backup.
- macOS behavior still requires hosted/physical Mac validation, especially File Provider hydration, permissions, app-server discovery, and atomic rename behavior.
- The fork intentionally does not publish Windows ARM, macOS Intel/universal, or Linux artifacts. Upstream changes to shared release workflows can conflict with this fork-only architecture policy during automatic upstream merges.
- Cockpit Tools is CC BY-NC-SA 4.0; commercial/internal business use requires separate authorization.
- Workspace-wide Clippy with `-D warnings` is blocked by more than 180 pre-existing upstream warnings. The new Drive-sync files have no Clippy warnings; two remaining warnings in `cockpit-cli/src/main.rs` are pre-existing lines last changed by upstream commit `1e2b1a9`.
- Rendered GUI interaction and responsive screenshots remain unverified because the in-app Browser runtime returned no available browser backend; build and source-level checks passed.
- The updater signing key backup is tied to the current Windows account through DPAPI. Preserve the encrypted private key and password recovery file together and migrate them deliberately before replacing this machine or account.
