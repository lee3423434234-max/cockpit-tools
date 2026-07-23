# Codex session synchronization through Google Drive

Cockpit CLI can exchange encrypted, immutable Codex session snapshots through a Google Drive for desktop folder. It does **not** synchronize `auth.json`, configuration, logs, SQLite databases, or all of `CODEX_HOME`.

## Safety model

- Every `rollout-*.jsonl` becomes a separate AES-256-GCM encrypted object.
- PBKDF2-HMAC-SHA256 derives object keys from a passphrase that is supplied through an environment variable and never written to Drive.
- Object names use the SHA-256 of canonical session content. Source-specific `cwd` and `model_provider` fields are excluded from the canonical hash so Windows/macOS path rewriting does not create a false conflict.
- Each device writes only its own head file. `.partial` files are ignored.
- A missing session is imported; an identical session is a no-op; a strict-prefix extension fast-forwards; divergent histories create a conflict record and never overwrite automatically.
- Imports require every Codex process to be stopped. Existing rollout and index files are backed up before replacement.
- After imports, Cockpit calls the official Codex app-server to rebuild thread metadata. A failed rebuild remains marked as pending in local-only state and is retried on the next run.

## First-device setup

Choose a private Google Drive directory and a strong shared passphrase. Keep the state file outside Google Drive.

PowerShell example:

```powershell
$env:COCKPIT_DRIVE_SYNC_ROOT = 'G:\My Drive\Codex Sessions AutoSync'
$env:COCKPIT_DRIVE_SYNC_PASSPHRASE = '<a strong passphrase stored in your password manager>'

cargo run -p cockpit-cli -- codex sessions status
cargo run -p cockpit-cli -- codex sessions sync-once --upload-only --dry-run
cargo run -p cockpit-cli -- codex sessions sync-once --upload-only
```

Use `--upload-only` on the first computer until the encrypted object and head counts look correct.

## Second-device setup

Use the same Drive folder and passphrase. Close Codex before importing. Map source paths to the destination computer when needed:

```powershell
cargo run -p cockpit-cli -- codex sessions sync-once --dry-run `
  --map-cwd 'C:\Users\Tom\Documents=/Users/tom/Documents' `
  --provider openai

cargo run -p cockpit-cli -- codex sessions sync-once `
  --map-cwd 'C:\Users\Tom\Documents=/Users/tom/Documents' `
  --provider openai
```

On macOS, Drive for desktop normally exposes the selected folder below `~/Library/CloudStorage`. Pass its absolute path through `COCKPIT_DRIVE_SYNC_ROOT`.

## Commands

```text
cockpit-cli codex sessions status
cockpit-cli codex sessions sync-once [--dry-run] [--upload-only]
cockpit-cli codex sessions daemon --interval-seconds 60
cockpit-cli codex sessions resolve-conflict --object-sha <sha256>
```

`resolve-conflict` is intentionally explicit and destructive: after reviewing the conflicting device heads, it accepts the selected encrypted object, backs up the local rollout and index, rewrites target metadata, rebuilds Codex metadata, and archives the conflict record.

## Drive layout

```text
Codex Sessions AutoSync/
  v1/
    objects/<canonical-sha256>.cxs
    heads/<sha256-session-id>/<device-id>.json
    conflicts/<sha256-session-id>/<fingerprint>.json
    conflicts/resolved/...
```

The local state defaults to the platform application-data directory under `cockpit-tools/codex-drive-sync/state.json`. Never move it into Google Drive.
