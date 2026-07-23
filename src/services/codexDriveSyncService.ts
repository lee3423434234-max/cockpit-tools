import { invoke } from '@tauri-apps/api/core';

export interface CodexDriveSyncStatus {
  codex_home: string;
  drive_root: string;
  state_path: string;
  codex_running: boolean;
  local_sessions: number;
  remote_objects: number;
  remote_heads: number;
  partial_files: number;
  conflicts: number;
  device_id: string | null;
  index_rebuild_pending: boolean;
}

export interface CodexDriveSyncSummary {
  device_id: string;
  local_sessions: number;
  uploaded_objects: number;
  published_heads: number;
  imported_sessions: number;
  fast_forwarded_sessions: number;
  no_op_sessions: number;
  conflicts: number;
  partial_files_ignored: number;
  index_rebuilt: boolean;
  index_rebuild_pending: boolean;
  dry_run: boolean;
  warnings: string[];
}

export interface CodexDriveSyncConflict {
  version: number;
  session_key: string;
  local_content_sha256: string | null;
  remote_content_sha256: string[];
  detected_at: string;
}

export interface CodexDriveSyncResolveSummary {
  session_key: string;
  object_sha256: string;
  target_path: string;
  index_rebuilt: boolean;
  dry_run: boolean;
}

export interface CodexDriveSyncRunRequest {
  driveRoot: string;
  passphrase: string;
  cwdMappings: string[];
  providerOverride?: string | null;
  dryRun: boolean;
  uploadOnly: boolean;
}

export interface CodexDriveSyncResolveRequest {
  driveRoot: string;
  passphrase: string;
  cwdMappings: string[];
  providerOverride?: string | null;
  objectSha256: string;
  dryRun: boolean;
}

export function getCodexDriveSyncStatus(driveRoot: string): Promise<CodexDriveSyncStatus> {
  return invoke<CodexDriveSyncStatus>('codex_drive_sync_status', { driveRoot });
}

export function runCodexDriveSync(
  request: CodexDriveSyncRunRequest,
): Promise<CodexDriveSyncSummary> {
  return invoke<CodexDriveSyncSummary>('codex_drive_sync_run', { request });
}

export function listCodexDriveSyncConflicts(
  driveRoot: string,
): Promise<CodexDriveSyncConflict[]> {
  return invoke<CodexDriveSyncConflict[]>('codex_drive_sync_list_conflicts', { driveRoot });
}

export function resolveCodexDriveSyncConflict(
  request: CodexDriveSyncResolveRequest,
): Promise<CodexDriveSyncResolveSummary> {
  return invoke<CodexDriveSyncResolveSummary>('codex_drive_sync_resolve_conflict', { request });
}
