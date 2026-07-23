use std::fs;
use std::path::{Path, PathBuf};

use cockpit_core::modules::codex_drive_sync::{
    default_codex_home, default_state_path, DriveSyncConfig, DriveSyncStatus, PathMapping,
    ResolveSummary, SyncEngine, SyncSummary,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexDriveSyncRequest {
    drive_root: String,
    passphrase: String,
    #[serde(default)]
    cwd_mappings: Vec<String>,
    provider_override: Option<String>,
    dry_run: bool,
    upload_only: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexDriveSyncResolveRequest {
    drive_root: String,
    passphrase: String,
    #[serde(default)]
    cwd_mappings: Vec<String>,
    provider_override: Option<String>,
    object_sha256: String,
    dry_run: bool,
}

#[derive(Clone, Deserialize)]
struct ConflictRecordFile {
    version: u32,
    session_key: String,
    local_content_sha256: Option<String>,
    #[serde(default)]
    remote_content_sha256: Vec<String>,
    detected_at: String,
}

#[derive(Clone, Serialize)]
pub struct CodexDriveSyncConflict {
    version: u32,
    session_key: String,
    local_content_sha256: Option<String>,
    remote_content_sha256: Vec<String>,
    detected_at: String,
}

fn normalize_non_empty(value: String, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is required"));
    }
    Ok(trimmed.to_string())
}

fn parse_mappings(values: Vec<String>) -> Result<Vec<PathMapping>, String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| PathMapping::parse(&value))
        .collect()
}

fn normalize_provider(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn build_config(
    drive_root: String,
    passphrase: String,
    cwd_mappings: Vec<String>,
    provider_override: Option<String>,
    dry_run: bool,
    upload_only: bool,
) -> Result<DriveSyncConfig, String> {
    Ok(DriveSyncConfig {
        codex_home: default_codex_home()?,
        drive_root: PathBuf::from(normalize_non_empty(drive_root, "Google Drive folder")?),
        state_path: default_state_path()?,
        passphrase,
        device_id: None,
        cwd_mappings: parse_mappings(cwd_mappings)?,
        provider_override: normalize_provider(provider_override),
        app_server_executable: None,
        dry_run,
        upload_only,
        skip_index_rebuild: false,
    })
}

fn status_config(drive_root: String) -> Result<DriveSyncConfig, String> {
    build_config(drive_root, String::new(), Vec::new(), None, true, true)
}

fn read_conflict_records(drive_root: &Path) -> Result<Vec<CodexDriveSyncConflict>, String> {
    let root = drive_root.join("v1").join("conflicts");
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut conflicts = Vec::new();
    let session_dirs = fs::read_dir(&root).map_err(|error| {
        format!(
            "unable to read conflict directory {}: {error}",
            root.display()
        )
    })?;
    for session_dir in session_dirs {
        let session_dir = session_dir
            .map_err(|error| format!("unable to read an entry in {}: {error}", root.display()))?;
        let path = session_dir.path();
        if !path.is_dir() || session_dir.file_name() == "resolved" {
            continue;
        }
        let records = fs::read_dir(&path).map_err(|error| {
            format!(
                "unable to read conflict records in {}: {error}",
                path.display()
            )
        })?;
        for record in records {
            let record = record.map_err(|error| {
                format!("unable to read an entry in {}: {error}", path.display())
            })?;
            let record_path = record.path();
            if record_path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let raw = fs::read_to_string(&record_path).map_err(|error| {
                format!(
                    "unable to read conflict record {}: {error}",
                    record_path.display()
                )
            })?;
            let parsed: ConflictRecordFile = serde_json::from_str(&raw).map_err(|error| {
                format!("invalid conflict record {}: {error}", record_path.display())
            })?;
            conflicts.push(CodexDriveSyncConflict {
                version: parsed.version,
                session_key: parsed.session_key,
                local_content_sha256: parsed.local_content_sha256,
                remote_content_sha256: parsed.remote_content_sha256,
                detected_at: parsed.detected_at,
            });
        }
    }
    conflicts.sort_by(|left, right| right.detected_at.cmp(&left.detected_at));
    Ok(conflicts)
}

#[tauri::command]
pub async fn codex_drive_sync_status(drive_root: String) -> Result<DriveSyncStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = SyncEngine::new(status_config(drive_root)?)?;
        engine.status()
    })
    .await
    .map_err(|error| format!("Codex Drive status task failed: {error}"))?
}

#[tauri::command]
pub async fn codex_drive_sync_run(request: CodexDriveSyncRequest) -> Result<SyncSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = build_config(
            request.drive_root,
            request.passphrase,
            request.cwd_mappings,
            request.provider_override,
            request.dry_run,
            request.upload_only,
        )?;
        SyncEngine::new(config)?.sync_once()
    })
    .await
    .map_err(|error| format!("Codex Drive sync task failed: {error}"))?
}

#[tauri::command]
pub async fn codex_drive_sync_list_conflicts(
    drive_root: String,
) -> Result<Vec<CodexDriveSyncConflict>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = status_config(drive_root)?;
        config.validate()?;
        read_conflict_records(&config.drive_root)
    })
    .await
    .map_err(|error| format!("Codex Drive conflict scan failed: {error}"))?
}

#[tauri::command]
pub async fn codex_drive_sync_resolve_conflict(
    request: CodexDriveSyncResolveRequest,
) -> Result<ResolveSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let config = build_config(
            request.drive_root,
            request.passphrase,
            request.cwd_mappings,
            request.provider_override,
            request.dry_run,
            false,
        )?;
        SyncEngine::new(config)?.resolve_conflict(request.object_sha256.trim())
    })
    .await
    .map_err(|error| format!("Codex Drive conflict resolution task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::{normalize_provider, parse_mappings};

    #[test]
    fn parses_gui_mapping_lines_and_ignores_blank_values() {
        let mappings = parse_mappings(vec![
            " C:\\Users\\Tom=/Users/tom ".to_string(),
            String::new(),
        ])
        .expect("mapping should parse");
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].from, "C:\\Users\\Tom");
        assert_eq!(mappings[0].to, "/Users/tom");
    }

    #[test]
    fn trims_optional_provider_override() {
        assert_eq!(
            normalize_provider(Some(" openai ".to_string())).as_deref(),
            Some("openai")
        );
        assert_eq!(normalize_provider(Some("   ".to_string())), None);
    }
}
