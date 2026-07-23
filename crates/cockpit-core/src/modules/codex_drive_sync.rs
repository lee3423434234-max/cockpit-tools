use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use chrono::Utc;
use pbkdf2::pbkdf2_hmac;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const FORMAT_VERSION: u32 = 1;
const MAGIC: &[u8; 8] = b"CDXSYNC1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const PBKDF2_ITERATIONS: u32 = 210_000;
const SESSION_INDEX_FILE: &str = "session_index.jsonl";
const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathMapping {
    pub from: String,
    pub to: String,
}

impl PathMapping {
    pub fn parse(value: &str) -> Result<Self, String> {
        let (from, to) = value
            .split_once('=')
            .ok_or_else(|| "cwd mapping must use FROM=TO".to_string())?;
        if from.trim().is_empty() || to.trim().is_empty() {
            return Err("cwd mapping FROM and TO must both be non-empty".to_string());
        }
        Ok(Self {
            from: from.trim().to_string(),
            to: to.trim().to_string(),
        })
    }

    fn apply(&self, input: &str) -> Option<String> {
        let windows_style =
            self.from.as_bytes().get(1) == Some(&b':') || self.from.starts_with("\\\\");
        let matches = if windows_style {
            input
                .get(..self.from.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&self.from))
        } else {
            input.starts_with(&self.from)
        };
        if !matches {
            return None;
        }
        let suffix = &input[self.from.len()..];
        if !suffix.is_empty() && !suffix.starts_with('/') && !suffix.starts_with('\\') {
            return None;
        }
        Some(format!("{}{}", self.to, suffix))
    }
}

#[derive(Debug, Clone)]
pub struct DriveSyncConfig {
    pub codex_home: PathBuf,
    pub drive_root: PathBuf,
    pub state_path: PathBuf,
    pub passphrase: String,
    pub device_id: Option<String>,
    pub cwd_mappings: Vec<PathMapping>,
    pub provider_override: Option<String>,
    pub app_server_executable: Option<PathBuf>,
    pub dry_run: bool,
    pub upload_only: bool,
    pub skip_index_rebuild: bool,
}

impl DriveSyncConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.codex_home.is_absolute() {
            return Err("CODEX_HOME must be an absolute path".to_string());
        }
        if !self.drive_root.is_absolute() {
            return Err("Drive sync root must be an absolute path".to_string());
        }
        if !self.state_path.is_absolute() {
            return Err("local sync state must use an absolute path".to_string());
        }
        if self.drive_root.starts_with(&self.codex_home) {
            return Err("Drive sync root must not be inside CODEX_HOME".to_string());
        }
        if self.state_path.starts_with(&self.drive_root) {
            return Err("local sync state must not be stored in Google Drive".to_string());
        }
        if let Some(device_id) = &self.device_id {
            validate_component(device_id, "device id")?;
        }
        Ok(())
    }
}

pub fn default_codex_home() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("CODEX_HOME") {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    dirs::home_dir()
        .map(|home| home.join(".codex"))
        .ok_or_else(|| "unable to resolve the user home directory".to_string())
}

pub fn default_state_path() -> Result<PathBuf, String> {
    dirs::data_local_dir()
        .or_else(dirs::home_dir)
        .map(|root| {
            root.join("cockpit-tools")
                .join("codex-drive-sync")
                .join("state.json")
        })
        .ok_or_else(|| "unable to resolve a local application data directory".to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionManifest {
    pub version: u32,
    pub session_id: String,
    pub session_key: String,
    pub content_sha256: String,
    pub raw_sha256: String,
    pub relative_path: String,
    pub archived: bool,
    pub byte_len: u64,
    pub created_at: String,
    pub cwd: Option<String>,
    pub model_provider: Option<String>,
    pub session_index_entry: Option<JsonValue>,
}

#[derive(Debug, Clone)]
struct SessionSnapshot {
    manifest: SessionManifest,
    rollout: Vec<u8>,
    canonical: Vec<u8>,
}

#[derive(Debug, Clone)]
struct LocalSession {
    snapshot: SessionSnapshot,
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceHead {
    version: u32,
    session_key: String,
    object_sha256: String,
    device_id: String,
    published_at: String,
    byte_len: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LocalState {
    version: u32,
    device_id: String,
    uploaded: BTreeMap<String, String>,
    applied: BTreeMap<String, String>,
    index_rebuild_pending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveSyncStatus {
    pub codex_home: PathBuf,
    pub drive_root: PathBuf,
    pub state_path: PathBuf,
    pub codex_running: bool,
    pub local_sessions: usize,
    pub remote_objects: usize,
    pub remote_heads: usize,
    pub partial_files: usize,
    pub conflicts: usize,
    pub device_id: Option<String>,
    pub index_rebuild_pending: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncSummary {
    pub device_id: String,
    pub local_sessions: usize,
    pub uploaded_objects: usize,
    pub published_heads: usize,
    pub imported_sessions: usize,
    pub fast_forwarded_sessions: usize,
    pub no_op_sessions: usize,
    pub conflicts: usize,
    pub partial_files_ignored: usize,
    pub index_rebuilt: bool,
    pub index_rebuild_pending: bool,
    pub dry_run: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveSummary {
    pub session_key: String,
    pub object_sha256: String,
    pub target_path: PathBuf,
    pub index_rebuilt: bool,
    pub dry_run: bool,
}

pub struct SyncEngine {
    config: DriveSyncConfig,
}

impl SyncEngine {
    pub fn new(config: DriveSyncConfig) -> Result<Self, String> {
        config.validate()?;
        Ok(Self { config })
    }

    pub fn status(&self) -> Result<DriveSyncStatus, String> {
        let local_sessions = scan_local_sessions(&self.config.codex_home)?.len();
        let state = load_state(&self.config.state_path)?;
        let transport_root = self.transport_root();
        Ok(DriveSyncStatus {
            codex_home: self.config.codex_home.clone(),
            drive_root: self.config.drive_root.clone(),
            state_path: self.config.state_path.clone(),
            codex_running: super::process::is_codex_running(),
            local_sessions,
            remote_objects: count_files_with_extension(&transport_root.join("objects"), "cxs")?,
            remote_heads: count_files_with_extension(&transport_root.join("heads"), "json")?,
            partial_files: count_partial_files(&transport_root)?,
            conflicts: count_files_with_extension(&transport_root.join("conflicts"), "json")?,
            device_id: state.as_ref().map(|item| item.device_id.clone()),
            index_rebuild_pending: state
                .as_ref()
                .is_some_and(|item| item.index_rebuild_pending),
        })
    }

    pub fn sync_once(&self) -> Result<SyncSummary, String> {
        self.sync_once_with_running(super::process::is_codex_running())
    }

    fn sync_once_with_running(&self, codex_running: bool) -> Result<SyncSummary, String> {
        self.require_passphrase()?;
        if codex_running && !self.config.dry_run {
            return Err(
                "Codex is running. Close every Codex instance before session synchronization."
                    .to_string(),
            );
        }

        let mut state = self.load_or_create_state()?;
        let device_id = self
            .config
            .device_id
            .clone()
            .unwrap_or_else(|| state.device_id.clone());
        validate_component(&device_id, "device id")?;

        if !self.config.dry_run {
            self.ensure_transport_layout()?;
        }

        let mut local_sessions = scan_local_sessions(&self.config.codex_home)?;
        let mut summary = SyncSummary {
            device_id: device_id.clone(),
            local_sessions: local_sessions.len(),
            partial_files_ignored: count_partial_files(&self.transport_root())?,
            dry_run: self.config.dry_run,
            ..SyncSummary::default()
        };

        for local in local_sessions.values() {
            let object_created = self.publish_object(&local.snapshot)?;
            if object_created {
                summary.uploaded_objects += 1;
            }
            if self.publish_head(&local.snapshot, &device_id)? {
                summary.published_heads += 1;
            }
            state.uploaded.insert(
                local.snapshot.manifest.session_id.clone(),
                local.snapshot.manifest.content_sha256.clone(),
            );
        }
        if !self.config.dry_run {
            save_state(&self.config.state_path, &state)?;
        }

        if self.config.upload_only {
            summary.index_rebuild_pending = state.index_rebuild_pending;
            return Ok(summary);
        }

        let remote_groups = self.load_remote_groups()?;
        let mut apply = Vec::<(SessionSnapshot, Option<LocalSession>, bool)>::new();
        for (session_id, snapshots) in remote_groups {
            let local = local_sessions.get(&session_id);
            match choose_remote(local, &snapshots) {
                ReconcileDecision::NoOp => summary.no_op_sessions += 1,
                ReconcileDecision::Apply(snapshot, is_fast_forward) => {
                    apply.push((snapshot, local.cloned(), is_fast_forward));
                }
                ReconcileDecision::Conflict(remote_hashes) => {
                    summary.conflicts += 1;
                    self.write_conflict(local, &snapshots, &remote_hashes)?;
                }
            }
        }

        if !apply.is_empty() {
            if !self.config.dry_run {
                state.index_rebuild_pending = true;
                save_state(&self.config.state_path, &state)?;
            }
            let backup_root = self.backup_root();
            let index_path = self.config.codex_home.join(SESSION_INDEX_FILE);
            if !self.config.dry_run && index_path.is_file() {
                let backup_index = backup_root.join(SESSION_INDEX_FILE);
                if let Some(parent) = backup_index.parent() {
                    fs::create_dir_all(parent)
                        .map_err(io_error("create backup directory", parent))?;
                }
                fs::copy(&index_path, &backup_index)
                    .map_err(io_error("backup session index", &index_path))?;
            }

            let mut index_entries = Vec::new();
            for (snapshot, local, is_fast_forward) in apply {
                let target_path = local
                    .as_ref()
                    .map(|item| item.path.clone())
                    .unwrap_or_else(|| self.import_target_path(&snapshot.manifest));
                let rewritten = rewrite_rollout(
                    &snapshot.rollout,
                    &self.config.cwd_mappings,
                    self.config.provider_override.as_deref(),
                )?;
                if sha256_hex(&canonicalize_rollout(&rewritten)?)
                    != snapshot.manifest.content_sha256
                {
                    return Err(format!(
                        "rewriting metadata changed canonical content for session {}",
                        snapshot.manifest.session_key
                    ));
                }

                if !self.config.dry_run {
                    if target_path.is_file() {
                        backup_rollout(&target_path, &self.config.codex_home, &backup_root)?;
                    }
                    write_bytes_atomic(&target_path, &rewritten)?;
                }

                if let Some(entry) = build_index_entry(
                    &snapshot.manifest,
                    &self.config.cwd_mappings,
                    self.config.provider_override.as_deref(),
                ) {
                    index_entries.push(entry);
                }
                state.applied.insert(
                    snapshot.manifest.session_id.clone(),
                    snapshot.manifest.content_sha256.clone(),
                );
                if is_fast_forward {
                    summary.fast_forwarded_sessions += 1;
                } else {
                    summary.imported_sessions += 1;
                }

                local_sessions.insert(
                    snapshot.manifest.session_id.clone(),
                    LocalSession {
                        snapshot,
                        path: target_path,
                    },
                );
            }

            if !self.config.dry_run && !index_entries.is_empty() {
                upsert_session_index(&self.config.codex_home, &index_entries)?;
            }
        }

        if !self.config.dry_run && state.index_rebuild_pending {
            if self.config.skip_index_rebuild {
                state.index_rebuild_pending = false;
            } else {
                match super::codex_app_server::rebuild_thread_metadata(
                    &self.config.codex_home,
                    self.config.app_server_executable.as_deref(),
                ) {
                    Ok(()) => {
                        summary.index_rebuilt = true;
                        state.index_rebuild_pending = false;
                    }
                    Err(error) => summary.warnings.push(format!(
                        "sessions were written, but Codex metadata rebuild is pending: {error}"
                    )),
                }
            }
            save_state(&self.config.state_path, &state)?;
        }
        summary.index_rebuild_pending = state.index_rebuild_pending;
        Ok(summary)
    }

    pub fn resolve_conflict(&self, object_sha256: &str) -> Result<ResolveSummary, String> {
        self.resolve_conflict_with_running(object_sha256, super::process::is_codex_running())
    }

    fn resolve_conflict_with_running(
        &self,
        object_sha256: &str,
        codex_running: bool,
    ) -> Result<ResolveSummary, String> {
        self.require_passphrase()?;
        validate_sha256(object_sha256)?;
        if codex_running && !self.config.dry_run {
            return Err("Codex is running; close it before resolving a conflict".to_string());
        }
        let object_path = self.object_path(object_sha256);
        let snapshot = decode_snapshot(
            &fs::read(&object_path).map_err(io_error("read encrypted object", &object_path))?,
            &self.config.passphrase,
        )?;
        if snapshot.manifest.content_sha256 != object_sha256 {
            return Err("selected object hash does not match its verified content".to_string());
        }

        let local_sessions = scan_local_sessions(&self.config.codex_home)?;
        let local = local_sessions.get(&snapshot.manifest.session_id);
        let target_path = local
            .map(|item| item.path.clone())
            .unwrap_or_else(|| self.import_target_path(&snapshot.manifest));
        let rewritten = rewrite_rollout(
            &snapshot.rollout,
            &self.config.cwd_mappings,
            self.config.provider_override.as_deref(),
        )?;

        let mut state = self.load_or_create_state()?;
        if !self.config.dry_run {
            state.index_rebuild_pending = true;
            save_state(&self.config.state_path, &state)?;
            let backup_root = self.backup_root();
            let index_path = self.config.codex_home.join(SESSION_INDEX_FILE);
            if index_path.is_file() {
                let backup_index = backup_root.join(SESSION_INDEX_FILE);
                if let Some(parent) = backup_index.parent() {
                    fs::create_dir_all(parent)
                        .map_err(io_error("create backup directory", parent))?;
                }
                fs::copy(&index_path, &backup_index)
                    .map_err(io_error("backup session index", &index_path))?;
            }
            if target_path.is_file() {
                backup_rollout(&target_path, &self.config.codex_home, &backup_root)?;
            }
            write_bytes_atomic(&target_path, &rewritten)?;
            if let Some(entry) = build_index_entry(
                &snapshot.manifest,
                &self.config.cwd_mappings,
                self.config.provider_override.as_deref(),
            ) {
                upsert_session_index(&self.config.codex_home, &[entry])?;
            }
            state.applied.insert(
                snapshot.manifest.session_id.clone(),
                snapshot.manifest.content_sha256.clone(),
            );
        }

        let mut index_rebuilt = false;
        if !self.config.dry_run {
            if self.config.skip_index_rebuild {
                state.index_rebuild_pending = false;
            } else {
                super::codex_app_server::rebuild_thread_metadata(
                    &self.config.codex_home,
                    self.config.app_server_executable.as_deref(),
                )?;
                state.index_rebuild_pending = false;
                index_rebuilt = true;
            }
            save_state(&self.config.state_path, &state)?;
            self.archive_conflict_records(&snapshot.manifest.session_key)?;
        }

        Ok(ResolveSummary {
            session_key: snapshot.manifest.session_key,
            object_sha256: object_sha256.to_string(),
            target_path,
            index_rebuilt,
            dry_run: self.config.dry_run,
        })
    }

    fn transport_root(&self) -> PathBuf {
        self.config.drive_root.join("v1")
    }

    fn ensure_transport_layout(&self) -> Result<(), String> {
        for name in ["objects", "heads", "conflicts", "conflicts/resolved"] {
            let path = self.transport_root().join(name);
            fs::create_dir_all(&path)
                .map_err(io_error("create Drive transport directory", &path))?;
        }
        Ok(())
    }

    fn require_passphrase(&self) -> Result<(), String> {
        if self.config.passphrase.len() < 12 {
            return Err(
                "sync passphrase must contain at least 12 characters and must be supplied outside Google Drive"
                    .to_string(),
            );
        }
        Ok(())
    }

    fn load_or_create_state(&self) -> Result<LocalState, String> {
        if let Some(mut state) = load_state(&self.config.state_path)? {
            if let Some(device_id) = &self.config.device_id {
                state.device_id = device_id.clone();
            }
            return Ok(state);
        }
        Ok(LocalState {
            version: FORMAT_VERSION,
            device_id: self
                .config
                .device_id
                .clone()
                .unwrap_or_else(|| format!("device-{}", Uuid::new_v4())),
            ..LocalState::default()
        })
    }

    fn object_path(&self, content_sha256: &str) -> PathBuf {
        self.transport_root()
            .join("objects")
            .join(format!("{content_sha256}.cxs"))
    }

    fn publish_object(&self, snapshot: &SessionSnapshot) -> Result<bool, String> {
        let path = self.object_path(&snapshot.manifest.content_sha256);
        if path.is_file() {
            let existing = decode_snapshot(
                &fs::read(&path).map_err(io_error("read existing Drive object", &path))?,
                &self.config.passphrase,
            )?;
            if existing.manifest.content_sha256 != snapshot.manifest.content_sha256 {
                return Err(format!(
                    "Drive object failed verification: {}",
                    path.display()
                ));
            }
            return Ok(false);
        }
        if self.config.dry_run {
            return Ok(true);
        }
        let encoded = encode_snapshot(snapshot, &self.config.passphrase)?;
        write_bytes_atomic(&path, &encoded)?;
        Ok(true)
    }

    fn publish_head(&self, snapshot: &SessionSnapshot, device_id: &str) -> Result<bool, String> {
        let path = self
            .transport_root()
            .join("heads")
            .join(&snapshot.manifest.session_key)
            .join(format!("{device_id}.json"));
        let head = DeviceHead {
            version: FORMAT_VERSION,
            session_key: snapshot.manifest.session_key.clone(),
            object_sha256: snapshot.manifest.content_sha256.clone(),
            device_id: device_id.to_string(),
            published_at: Utc::now().to_rfc3339(),
            byte_len: snapshot.manifest.byte_len,
        };
        if path.is_file() {
            let existing = serde_json::from_slice::<DeviceHead>(
                &fs::read(&path).map_err(io_error("read Drive head", &path))?,
            )
            .map_err(|error| format!("invalid Drive head {}: {error}", path.display()))?;
            if existing.object_sha256 == head.object_sha256 {
                return Ok(false);
            }
        }
        if !self.config.dry_run {
            write_json_atomic(&path, &head)?;
        }
        Ok(true)
    }

    fn load_remote_groups(&self) -> Result<HashMap<String, Vec<SessionSnapshot>>, String> {
        let heads_root = self.transport_root().join("heads");
        let mut groups = HashMap::<String, Vec<SessionSnapshot>>::new();
        let mut cache = HashMap::<String, SessionSnapshot>::new();
        for path in list_files_recursive(&heads_root)? {
            if path.extension().and_then(|value| value.to_str()) != Some("json")
                || is_partial_path(&path)
            {
                continue;
            }
            let head = serde_json::from_slice::<DeviceHead>(
                &fs::read(&path).map_err(io_error("read Drive head", &path))?,
            )
            .map_err(|error| format!("invalid Drive head {}: {error}", path.display()))?;
            if head.version != FORMAT_VERSION {
                return Err(format!(
                    "unsupported Drive head version {} in {}",
                    head.version,
                    path.display()
                ));
            }
            validate_sha256(&head.session_key)?;
            validate_sha256(&head.object_sha256)?;
            let object_path = self.object_path(&head.object_sha256);
            if !object_path.is_file() {
                // Google Drive can expose the head before its immutable object.
                continue;
            }
            let snapshot = if let Some(snapshot) = cache.get(&head.object_sha256) {
                snapshot.clone()
            } else {
                let decoded = decode_snapshot(
                    &fs::read(&object_path)
                        .map_err(io_error("read encrypted Drive object", &object_path))?,
                    &self.config.passphrase,
                )?;
                if decoded.manifest.content_sha256 != head.object_sha256
                    || decoded.manifest.session_key != head.session_key
                {
                    return Err(format!(
                        "Drive head/object verification mismatch: {}",
                        path.display()
                    ));
                }
                cache.insert(head.object_sha256.clone(), decoded.clone());
                decoded
            };
            groups
                .entry(snapshot.manifest.session_id.clone())
                .or_default()
                .push(snapshot);
        }
        Ok(groups)
    }

    fn write_conflict(
        &self,
        local: Option<&LocalSession>,
        snapshots: &[SessionSnapshot],
        remote_hashes: &[String],
    ) -> Result<(), String> {
        if self.config.dry_run || snapshots.is_empty() {
            return Ok(());
        }
        let session_key = &snapshots[0].manifest.session_key;
        let local_hash = local
            .map(|item| item.snapshot.manifest.content_sha256.clone())
            .unwrap_or_default();
        let mut remote_hashes = remote_hashes.to_vec();
        remote_hashes.sort();
        remote_hashes.dedup();
        let fingerprint =
            sha256_hex(format!("{local_hash}:{}", remote_hashes.join(":")).as_bytes());
        let path = self
            .transport_root()
            .join("conflicts")
            .join(session_key)
            .join(format!("{fingerprint}.json"));
        if path.is_file() {
            return Ok(());
        }
        write_json_atomic(
            &path,
            &json!({
                "version": FORMAT_VERSION,
                "session_key": session_key,
                "local_content_sha256": if local_hash.is_empty() { JsonValue::Null } else { JsonValue::String(local_hash) },
                "remote_content_sha256": remote_hashes,
                "detected_at": Utc::now().to_rfc3339(),
                "resolution": "Run `cockpit-cli codex sessions resolve-conflict --object-sha <sha256>` after reviewing the encrypted object source.",
            }),
        )
    }

    fn import_target_path(&self, manifest: &SessionManifest) -> PathBuf {
        let now = Utc::now();
        let root = if manifest.archived {
            "archived_sessions"
        } else {
            "sessions"
        };
        self.config
            .codex_home
            .join(root)
            .join("imported")
            .join(now.format("%Y").to_string())
            .join(now.format("%m").to_string())
            .join(now.format("%d").to_string())
            .join(format!(
                "rollout-{}.jsonl",
                safe_file_component(&manifest.session_id)
            ))
    }

    fn backup_root(&self) -> PathBuf {
        self.config
            .codex_home
            .join("backups")
            .join("codex-drive-sync")
            .join(Utc::now().format("%Y%m%d-%H%M%S%.3f").to_string())
    }

    fn archive_conflict_records(&self, session_key: &str) -> Result<(), String> {
        let source = self.transport_root().join("conflicts").join(session_key);
        if !source.is_dir() {
            return Ok(());
        }
        let target = self
            .transport_root()
            .join("conflicts")
            .join("resolved")
            .join(format!(
                "{}-{}",
                session_key,
                Utc::now().format("%Y%m%d-%H%M%S%.3f")
            ));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(io_error("create conflict archive", parent))?;
        }
        fs::rename(&source, &target).map_err(io_error("archive conflict records", &source))
    }
}

#[derive(Debug)]
enum ReconcileDecision {
    NoOp,
    Apply(SessionSnapshot, bool),
    Conflict(Vec<String>),
}

fn choose_remote(local: Option<&LocalSession>, snapshots: &[SessionSnapshot]) -> ReconcileDecision {
    let mut unique = HashMap::<String, SessionSnapshot>::new();
    for snapshot in snapshots {
        unique
            .entry(snapshot.manifest.content_sha256.clone())
            .or_insert_with(|| snapshot.clone());
    }
    let mut candidates = unique.into_values().collect::<Vec<_>>();
    candidates.sort_by_key(|item| item.canonical.len());
    let hashes = candidates
        .iter()
        .map(|item| item.manifest.content_sha256.clone())
        .collect::<Vec<_>>();
    let Some(longest) = candidates.last().cloned() else {
        return ReconcileDecision::NoOp;
    };
    if candidates
        .iter()
        .any(|candidate| !longest.canonical.starts_with(&candidate.canonical))
    {
        return ReconcileDecision::Conflict(hashes);
    }
    let Some(local) = local else {
        return ReconcileDecision::Apply(longest, false);
    };
    if local.snapshot.canonical == longest.canonical
        || local.snapshot.canonical.starts_with(&longest.canonical)
    {
        return ReconcileDecision::NoOp;
    }
    if longest.canonical.starts_with(&local.snapshot.canonical) {
        return ReconcileDecision::Apply(longest, true);
    }
    ReconcileDecision::Conflict(hashes)
}

fn scan_local_sessions(codex_home: &Path) -> Result<HashMap<String, LocalSession>, String> {
    let index = read_session_index(codex_home)?;
    let mut sessions = HashMap::<String, LocalSession>::new();
    for session_dir in SESSION_DIRS {
        let root = codex_home.join(session_dir);
        for path in list_files_recursive(&root)? {
            let file_name = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if !file_name.starts_with("rollout-") || !file_name.ends_with(".jsonl") {
                continue;
            }
            let raw = fs::read(&path).map_err(io_error("read rollout", &path))?;
            let meta = read_session_meta(&raw)?;
            let session_id = session_meta_string(&meta, &["id", "session_id"])
                .ok_or_else(|| format!("rollout has no session id: {}", path.display()))?;
            let canonical = canonicalize_rollout(&raw)?;
            let content_sha256 = sha256_hex(&canonical);
            let relative = path
                .strip_prefix(codex_home)
                .map_err(|_| format!("rollout escaped CODEX_HOME: {}", path.display()))?;
            validate_relative_path(relative)?;
            let snapshot = SessionSnapshot {
                manifest: SessionManifest {
                    version: FORMAT_VERSION,
                    session_key: sha256_hex(session_id.as_bytes()),
                    raw_sha256: sha256_hex(&raw),
                    content_sha256,
                    relative_path: path_to_slash(relative),
                    archived: session_dir == "archived_sessions",
                    byte_len: raw.len() as u64,
                    created_at: Utc::now().to_rfc3339(),
                    cwd: session_meta_string(&meta, &["cwd"]),
                    model_provider: session_meta_string(&meta, &["model_provider"]),
                    session_index_entry: index.get(&session_id).cloned(),
                    session_id: session_id.clone(),
                },
                rollout: raw,
                canonical,
            };
            let candidate = LocalSession { snapshot, path };
            match sessions.get(&session_id) {
                None => {
                    sessions.insert(session_id, candidate);
                }
                Some(existing)
                    if candidate
                        .snapshot
                        .canonical
                        .starts_with(&existing.snapshot.canonical) =>
                {
                    sessions.insert(session_id, candidate);
                }
                Some(existing)
                    if existing
                        .snapshot
                        .canonical
                        .starts_with(&candidate.snapshot.canonical) => {}
                Some(_) => {
                    return Err(format!(
                        "divergent duplicate local rollouts found for session {session_id}"
                    ));
                }
            }
        }
    }
    Ok(sessions)
}

fn read_session_meta(raw: &[u8]) -> Result<JsonValue, String> {
    let line_end = raw
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(raw.len());
    let line = trim_cr(&raw[..line_end]);
    let value = serde_json::from_slice::<JsonValue>(line)
        .map_err(|error| format!("invalid rollout session_meta JSON: {error}"))?;
    if value.get("type").and_then(JsonValue::as_str) != Some("session_meta") {
        return Err("rollout first line is not session_meta".to_string());
    }
    Ok(value)
}

fn session_meta_string(meta: &JsonValue, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = meta
            .get("payload")
            .and_then(|payload| payload.get(*key))
            .or_else(|| meta.get(*key))
            .and_then(JsonValue::as_str)
        {
            if !value.trim().is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn canonicalize_rollout(raw: &[u8]) -> Result<Vec<u8>, String> {
    let line_end = raw
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(raw.len());
    let mut meta = read_session_meta(raw)?;
    remove_session_meta_field(&mut meta, "cwd");
    remove_session_meta_field(&mut meta, "model_provider");
    let mut output = serde_json::to_vec(&meta)
        .map_err(|error| format!("failed to canonicalize session_meta: {error}"))?;
    if line_end < raw.len() {
        output.push(b'\n');
        output.extend_from_slice(&raw[line_end + 1..]);
    }
    Ok(output)
}

fn remove_session_meta_field(meta: &mut JsonValue, key: &str) {
    if let Some(object) = meta.as_object_mut() {
        object.remove(key);
        if let Some(payload) = object.get_mut("payload").and_then(JsonValue::as_object_mut) {
            payload.remove(key);
        }
    }
}

fn rewrite_rollout(
    raw: &[u8],
    mappings: &[PathMapping],
    provider_override: Option<&str>,
) -> Result<Vec<u8>, String> {
    let line_end = raw
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(raw.len());
    let mut meta = read_session_meta(raw)?;
    let current_cwd = session_meta_string(&meta, &["cwd"]);
    let mapped_cwd = current_cwd
        .as_deref()
        .and_then(|cwd| mappings.iter().find_map(|mapping| mapping.apply(cwd)));
    if let Some(cwd) = mapped_cwd.as_deref() {
        set_session_meta_field(&mut meta, "cwd", cwd);
    }
    if let Some(provider) = provider_override.filter(|value| !value.trim().is_empty()) {
        set_session_meta_field(&mut meta, "model_provider", provider.trim());
    }
    let mut output = serde_json::to_vec(&meta)
        .map_err(|error| format!("failed to rewrite session_meta: {error}"))?;
    if line_end < raw.len() {
        output.push(b'\n');
        output.extend_from_slice(&raw[line_end + 1..]);
    }
    Ok(output)
}

fn set_session_meta_field(meta: &mut JsonValue, key: &str, value: &str) {
    if !meta.is_object() {
        *meta = JsonValue::Object(JsonMap::new());
    }
    let object = meta.as_object_mut().expect("object initialized");
    if let Some(payload) = object.get_mut("payload").and_then(JsonValue::as_object_mut) {
        payload.insert(key.to_string(), JsonValue::String(value.to_string()));
    } else {
        object.insert(key.to_string(), JsonValue::String(value.to_string()));
    }
}

fn encode_snapshot(snapshot: &SessionSnapshot, passphrase: &str) -> Result<Vec<u8>, String> {
    let manifest = serde_json::to_vec(&snapshot.manifest)
        .map_err(|error| format!("failed to serialize snapshot manifest: {error}"))?;
    let manifest_len =
        u32::try_from(manifest.len()).map_err(|_| "snapshot manifest is too large".to_string())?;
    let mut plaintext = Vec::with_capacity(4 + manifest.len() + snapshot.rollout.len());
    plaintext.extend_from_slice(&manifest_len.to_be_bytes());
    plaintext.extend_from_slice(&manifest);
    plaintext.extend_from_slice(&snapshot.rollout);

    let mut salt = [0u8; SALT_LEN];
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce_bytes);
    let key = derive_key(passphrase, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| "failed to initialize AES-256-GCM".to_string())?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: &plaintext,
                aad: MAGIC,
            },
        )
        .map_err(|_| "failed to encrypt session snapshot".to_string())?;
    let mut output = Vec::with_capacity(MAGIC.len() + SALT_LEN + NONCE_LEN + ciphertext.len());
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce_bytes);
    output.extend_from_slice(&ciphertext);
    Ok(output)
}

fn decode_snapshot(encoded: &[u8], passphrase: &str) -> Result<SessionSnapshot, String> {
    let header_len = MAGIC.len() + SALT_LEN + NONCE_LEN;
    if encoded.len() <= header_len || &encoded[..MAGIC.len()] != MAGIC {
        return Err("invalid encrypted session object header".to_string());
    }
    let salt_start = MAGIC.len();
    let nonce_start = salt_start + SALT_LEN;
    let ciphertext_start = nonce_start + NONCE_LEN;
    let salt = &encoded[salt_start..nonce_start];
    let nonce = &encoded[nonce_start..ciphertext_start];
    let key = derive_key(passphrase, salt);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| "failed to initialize AES-256-GCM".to_string())?;
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: &encoded[ciphertext_start..],
                aad: MAGIC,
            },
        )
        .map_err(|_| {
            "unable to decrypt session object; passphrase or data is invalid".to_string()
        })?;
    if plaintext.len() < 4 {
        return Err("decrypted session object is truncated".to_string());
    }
    let manifest_len = u32::from_be_bytes(
        plaintext[..4]
            .try_into()
            .map_err(|_| "invalid manifest length".to_string())?,
    ) as usize;
    if manifest_len == 0 || plaintext.len() < 4 + manifest_len {
        return Err("decrypted session manifest is truncated".to_string());
    }
    let manifest = serde_json::from_slice::<SessionManifest>(&plaintext[4..4 + manifest_len])
        .map_err(|error| format!("invalid decrypted session manifest: {error}"))?;
    if manifest.version != FORMAT_VERSION {
        return Err(format!(
            "unsupported session object version {}",
            manifest.version
        ));
    }
    let rollout = plaintext[4 + manifest_len..].to_vec();
    if rollout.len() as u64 != manifest.byte_len || sha256_hex(&rollout) != manifest.raw_sha256 {
        return Err("decrypted rollout size or raw hash mismatch".to_string());
    }
    let canonical = canonicalize_rollout(&rollout)?;
    if sha256_hex(&canonical) != manifest.content_sha256
        || sha256_hex(manifest.session_id.as_bytes()) != manifest.session_key
    {
        return Err("decrypted rollout canonical hash or session key mismatch".to_string());
    }
    Ok(SessionSnapshot {
        manifest,
        rollout,
        canonical,
    })
}

fn derive_key(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), salt, PBKDF2_ITERATIONS, &mut key);
    key
}

fn read_session_index(codex_home: &Path) -> Result<HashMap<String, JsonValue>, String> {
    let path = codex_home.join(SESSION_INDEX_FILE);
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let file = fs::File::open(&path).map_err(io_error("open session index", &path))?;
    let mut entries = HashMap::new();
    for line in BufReader::new(file).lines() {
        let line =
            line.map_err(|error| format!("read session index {}: {error}", path.display()))?;
        let Ok(value) = serde_json::from_str::<JsonValue>(line.trim()) else {
            continue;
        };
        if let Some(id) = value.get("id").and_then(JsonValue::as_str) {
            entries.insert(id.to_string(), value);
        }
    }
    Ok(entries)
}

fn build_index_entry(
    manifest: &SessionManifest,
    mappings: &[PathMapping],
    _provider_override: Option<&str>,
) -> Option<JsonValue> {
    let mut entry = manifest.session_index_entry.clone().unwrap_or_else(|| {
        json!({
            "id": manifest.session_id,
            "thread_name": manifest.session_id,
            "updated_at": manifest.created_at,
        })
    });
    let mapped_cwd = manifest
        .cwd
        .as_deref()
        .and_then(|cwd| mappings.iter().find_map(|mapping| mapping.apply(cwd)))
        .or_else(|| manifest.cwd.clone());
    if let (Some(object), Some(cwd)) = (entry.as_object_mut(), mapped_cwd) {
        let keys = [
            "cwd",
            "workspace_root",
            "workspaceRoot",
            "working_directory",
            "workingDirectory",
        ];
        let mut changed = false;
        for key in keys {
            if object.contains_key(key) {
                object.insert(key.to_string(), JsonValue::String(cwd.clone()));
                changed = true;
            }
        }
        if !changed {
            object.insert("cwd".to_string(), JsonValue::String(cwd));
        }
    }
    Some(entry)
}

fn upsert_session_index(codex_home: &Path, replacements: &[JsonValue]) -> Result<(), String> {
    if replacements.is_empty() {
        return Ok(());
    }
    let path = codex_home.join(SESSION_INDEX_FILE);
    let existing = if path.is_file() {
        fs::read_to_string(&path).map_err(io_error("read session index", &path))?
    } else {
        String::new()
    };
    let mut replacement_map = BTreeMap::<String, String>::new();
    for replacement in replacements {
        let id = replacement
            .get("id")
            .and_then(JsonValue::as_str)
            .ok_or("replacement session index entry has no id")?;
        replacement_map.insert(
            id.to_string(),
            serde_json::to_string(replacement)
                .map_err(|error| format!("serialize session index entry: {error}"))?,
        );
    }
    let mut seen = HashSet::new();
    let mut lines = Vec::new();
    for line in existing.lines() {
        let replacement = serde_json::from_str::<JsonValue>(line.trim())
            .ok()
            .and_then(|value| {
                value
                    .get("id")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
            })
            .and_then(|id| {
                replacement_map.get(&id).map(|value| {
                    seen.insert(id);
                    value.clone()
                })
            });
        lines.push(replacement.unwrap_or_else(|| line.to_string()));
    }
    for (id, replacement) in replacement_map {
        if !seen.contains(&id) {
            lines.push(replacement);
        }
    }
    let mut output = lines.join("\n");
    output.push('\n');
    write_bytes_atomic(&path, output.as_bytes())
}

fn load_state(path: &Path) -> Result<Option<LocalState>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let state = serde_json::from_slice::<LocalState>(
        &fs::read(path).map_err(io_error("read local sync state", path))?,
    )
    .map_err(|error| format!("invalid local sync state {}: {error}", path.display()))?;
    if state.version != FORMAT_VERSION {
        return Err(format!(
            "unsupported local sync state version {}",
            state.version
        ));
    }
    validate_component(&state.device_id, "stored device id")?;
    Ok(Some(state))
}

fn save_state(path: &Path, state: &LocalState) -> Result<(), String> {
    write_json_atomic(path, state)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let mut data = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("serialize JSON for {}: {error}", path.display()))?;
    data.push(b'\n');
    write_bytes_atomic(path, &data)
}

fn write_bytes_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("target has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(io_error("create target directory", parent))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let temp = parent.join(format!(
        ".{file_name}.partial.{}.{}",
        std::process::id(),
        Uuid::new_v4()
    ));
    fs::write(&temp, data).map_err(io_error("write temporary file", &temp))?;
    if let Err(error) = atomic_replace(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(temp: &Path, target: &Path) -> Result<(), String> {
    fs::rename(temp, target).map_err(io_error("atomically replace file", target))
}

#[cfg(windows)]
fn atomic_replace(temp: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = temp
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| format!("atomically replace {}: {error}", target.display()))
}

fn backup_rollout(source: &Path, codex_home: &Path, backup_root: &Path) -> Result<(), String> {
    let relative = source.strip_prefix(codex_home).map_err(|_| {
        format!(
            "refusing to back up rollout outside CODEX_HOME: {}",
            source.display()
        )
    })?;
    validate_relative_path(relative)?;
    let target = backup_root.join(relative);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(io_error("create rollout backup directory", parent))?;
    }
    fs::copy(source, &target).map_err(io_error("backup rollout", source))?;
    Ok(())
}

fn list_files_recursive(root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    if !root.is_dir() {
        return Err(format!("expected directory: {}", root.display()));
    }
    let mut output = Vec::new();
    for entry in fs::read_dir(root).map_err(io_error("read directory", root))? {
        let entry = entry.map_err(|error| format!("read directory {}: {error}", root.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("read file type {}: {error}", path.display()))?;
        if file_type.is_dir() {
            output.extend(list_files_recursive(&path)?);
        } else if file_type.is_file() {
            output.push(path);
        }
    }
    output.sort();
    Ok(output)
}

fn count_files_with_extension(root: &Path, extension: &str) -> Result<usize, String> {
    Ok(list_files_recursive(root)?
        .into_iter()
        .filter(|path| {
            path.extension().and_then(|value| value.to_str()) == Some(extension)
                && !is_partial_path(path)
                && !path
                    .components()
                    .any(|component| component.as_os_str() == "resolved")
        })
        .count())
}

fn count_partial_files(root: &Path) -> Result<usize, String> {
    Ok(list_files_recursive(root)?
        .into_iter()
        .filter(|path| is_partial_path(path))
        .count())
}

fn is_partial_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.contains(".partial"))
}

fn validate_relative_path(path: &Path) -> Result<(), String> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe relative path: {}", path.display()));
    }
    Ok(())
}

fn path_to_slash(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str().map(str::to_string),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn validate_component(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!("invalid {label}: {value}"));
    }
    Ok(())
}

fn safe_file_component(value: &str) -> String {
    let safe = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if safe.is_empty() {
        sha256_hex(value.as_bytes())
    } else {
        safe.chars().take(120).collect()
    }
}

fn validate_sha256(value: &str) -> Result<(), String> {
    if value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(format!("invalid SHA-256 value: {value}"))
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn trim_cr(value: &[u8]) -> &[u8] {
    value.strip_suffix(b"\r").unwrap_or(value)
}

fn io_error<'a>(
    action: &'static str,
    path: &'a Path,
) -> impl FnOnce(std::io::Error) -> String + 'a {
    move |error| format!("{action} {}: {error}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const PASSPHRASE: &str = "correct horse battery staple";

    fn rollout(id: &str, cwd: &str, provider: &str, events: &[&str]) -> Vec<u8> {
        let mut value = format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\",\"model_provider\":\"{provider}\"}}}}\n"
        );
        for event in events {
            value.push_str(&format!(
                "{{\"type\":\"event\",\"payload\":{{\"text\":\"{event}\"}}}}\n"
            ));
        }
        value.into_bytes()
    }

    fn write_rollout(home: &Path, id: &str, bytes: &[u8]) -> PathBuf {
        let path = home
            .join("sessions/2026/07/23")
            .join(format!("rollout-{id}.jsonl"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    fn config(root: &Path, name: &str) -> DriveSyncConfig {
        DriveSyncConfig {
            codex_home: root.join(name).join(".codex"),
            drive_root: root.join("drive"),
            state_path: root.join(name).join("state.json"),
            passphrase: PASSPHRASE.to_string(),
            device_id: Some(name.to_string()),
            cwd_mappings: Vec::new(),
            provider_override: None,
            app_server_executable: None,
            dry_run: false,
            upload_only: false,
            skip_index_rebuild: true,
        }
    }

    #[test]
    fn encrypted_snapshot_round_trip_and_wrong_key_rejection() {
        let raw = rollout("s1", "/tmp/a", "openai", &["one"]);
        let canonical = canonicalize_rollout(&raw).unwrap();
        let snapshot = SessionSnapshot {
            manifest: SessionManifest {
                version: FORMAT_VERSION,
                session_id: "s1".into(),
                session_key: sha256_hex(b"s1"),
                content_sha256: sha256_hex(&canonical),
                raw_sha256: sha256_hex(&raw),
                relative_path: "sessions/rollout-s1.jsonl".into(),
                archived: false,
                byte_len: raw.len() as u64,
                created_at: Utc::now().to_rfc3339(),
                cwd: Some("/tmp/a".into()),
                model_provider: Some("openai".into()),
                session_index_entry: None,
            },
            rollout: raw,
            canonical,
        };
        let encrypted = encode_snapshot(&snapshot, PASSPHRASE).unwrap();
        let decoded = decode_snapshot(&encrypted, PASSPHRASE).unwrap();
        assert_eq!(decoded.rollout, snapshot.rollout);
        assert!(decode_snapshot(&encrypted, "this is the wrong passphrase").is_err());
        let mut tampered = encrypted;
        *tampered.last_mut().unwrap() ^= 0x01;
        assert!(decode_snapshot(&tampered, PASSPHRASE).is_err());
    }

    #[test]
    fn canonical_hash_ignores_target_specific_metadata() {
        let windows = rollout("s1", "C:/work/project", "relay", &["one"]);
        let mac = rollout("s1", "/Users/tom/project", "openai", &["one"]);
        assert_eq!(
            sha256_hex(&canonicalize_rollout(&windows).unwrap()),
            sha256_hex(&canonicalize_rollout(&mac).unwrap())
        );
    }

    #[test]
    fn new_session_import_is_idempotent() {
        let temp = TempDir::new().unwrap();
        let source = config(temp.path(), "source");
        let target = config(temp.path(), "target");
        write_rollout(
            &source.codex_home,
            "s1",
            &rollout("s1", "/source", "openai", &["one"]),
        );
        let source_summary = SyncEngine::new(source)
            .unwrap()
            .sync_once_with_running(false)
            .unwrap();
        assert_eq!(source_summary.uploaded_objects, 1);

        let engine = SyncEngine::new(target).unwrap();
        let first = engine.sync_once_with_running(false).unwrap();
        assert_eq!(first.imported_sessions, 1);
        let second = engine.sync_once_with_running(false).unwrap();
        assert_eq!(second.imported_sessions, 0);
        assert!(second.no_op_sessions >= 1);
    }

    #[test]
    fn strict_prefix_fast_forward_updates_existing_rollout() {
        let temp = TempDir::new().unwrap();
        let source = config(temp.path(), "source");
        let target = config(temp.path(), "target");
        write_rollout(
            &target.codex_home,
            "s1",
            &rollout("s1", "/target", "openai", &["one"]),
        );
        write_rollout(
            &source.codex_home,
            "s1",
            &rollout("s1", "/source", "relay", &["one", "two"]),
        );
        SyncEngine::new(source)
            .unwrap()
            .sync_once_with_running(false)
            .unwrap();
        let summary = SyncEngine::new(target.clone())
            .unwrap()
            .sync_once_with_running(false)
            .unwrap();
        assert_eq!(summary.fast_forwarded_sessions, 1);
        let sessions = scan_local_sessions(&target.codex_home).unwrap();
        assert!(String::from_utf8_lossy(&sessions["s1"].snapshot.rollout).contains("two"));
    }

    #[test]
    fn divergent_session_creates_conflict_without_overwrite() {
        let temp = TempDir::new().unwrap();
        let source = config(temp.path(), "source");
        let target = config(temp.path(), "target");
        let target_path = write_rollout(
            &target.codex_home,
            "s1",
            &rollout("s1", "/target", "openai", &["target"]),
        );
        let original = fs::read(&target_path).unwrap();
        write_rollout(
            &source.codex_home,
            "s1",
            &rollout("s1", "/source", "relay", &["source"]),
        );
        SyncEngine::new(source)
            .unwrap()
            .sync_once_with_running(false)
            .unwrap();
        let summary = SyncEngine::new(target)
            .unwrap()
            .sync_once_with_running(false)
            .unwrap();
        assert_eq!(summary.conflicts, 1);
        assert_eq!(fs::read(target_path).unwrap(), original);
        assert_eq!(
            count_files_with_extension(&temp.path().join("drive/v1/conflicts"), "json").unwrap(),
            1
        );
    }

    #[test]
    fn mapping_and_provider_rewrite_preserve_canonical_hash() {
        let raw = rollout("s1", "C:/work/project", "relay", &["one"]);
        let rewritten = rewrite_rollout(
            &raw,
            &[PathMapping {
                from: "C:/work".into(),
                to: "/Users/tom/work".into(),
            }],
            Some("openai"),
        )
        .unwrap();
        let text = String::from_utf8(rewritten.clone()).unwrap();
        assert!(text.contains("/Users/tom/work/project"));
        assert!(text.contains("\"model_provider\":\"openai\""));
        assert_eq!(
            sha256_hex(&canonicalize_rollout(&raw).unwrap()),
            sha256_hex(&canonicalize_rollout(&rewritten).unwrap())
        );
    }

    #[test]
    fn partial_drive_files_are_ignored() {
        let temp = TempDir::new().unwrap();
        let cfg = config(temp.path(), "target");
        let partial = cfg.drive_root.join("v1/objects/abc.cxs.partial.test");
        fs::create_dir_all(partial.parent().unwrap()).unwrap();
        fs::write(&partial, b"partial").unwrap();
        let summary = SyncEngine::new(cfg)
            .unwrap()
            .sync_once_with_running(false)
            .unwrap();
        assert_eq!(summary.partial_files_ignored, 1);
    }

    #[test]
    fn running_codex_blocks_all_writes() {
        let temp = TempDir::new().unwrap();
        let cfg = config(temp.path(), "target");
        write_rollout(
            &cfg.codex_home,
            "s1",
            &rollout("s1", "/target", "openai", &["one"]),
        );
        let error = SyncEngine::new(cfg.clone())
            .unwrap()
            .sync_once_with_running(true)
            .unwrap_err();
        assert!(error.contains("Codex is running"));
        assert!(!cfg.drive_root.exists());
        assert!(!cfg.state_path.exists());
    }

    #[test]
    fn upload_only_never_imports_remote_content() {
        let temp = TempDir::new().unwrap();
        let source = config(temp.path(), "source");
        write_rollout(
            &source.codex_home,
            "s1",
            &rollout("s1", "/source", "openai", &["one"]),
        );
        SyncEngine::new(source)
            .unwrap()
            .sync_once_with_running(false)
            .unwrap();

        let mut target = config(temp.path(), "target");
        target.upload_only = true;
        let summary = SyncEngine::new(target.clone())
            .unwrap()
            .sync_once_with_running(false)
            .unwrap();
        assert_eq!(summary.imported_sessions, 0);
        assert!(scan_local_sessions(&target.codex_home).unwrap().is_empty());
    }

    #[test]
    fn explicit_conflict_resolution_accepts_selected_object() {
        let temp = TempDir::new().unwrap();
        let source = config(temp.path(), "source");
        let target = config(temp.path(), "target");
        let source_raw = rollout("s1", "/source", "relay", &["source"]);
        let source_sha = sha256_hex(&canonicalize_rollout(&source_raw).unwrap());
        write_rollout(&source.codex_home, "s1", &source_raw);
        let target_path = write_rollout(
            &target.codex_home,
            "s1",
            &rollout("s1", "/target", "openai", &["target"]),
        );
        SyncEngine::new(source)
            .unwrap()
            .sync_once_with_running(false)
            .unwrap();
        let engine = SyncEngine::new(target).unwrap();
        assert_eq!(engine.sync_once_with_running(false).unwrap().conflicts, 1);
        let resolved = engine
            .resolve_conflict_with_running(&source_sha, false)
            .unwrap();
        assert_eq!(resolved.target_path, target_path);
        assert!(String::from_utf8_lossy(&fs::read(target_path).unwrap()).contains("source"));
    }

    #[test]
    fn state_path_inside_drive_is_rejected() {
        let temp = TempDir::new().unwrap();
        let mut cfg = config(temp.path(), "target");
        cfg.state_path = cfg.drive_root.join("state.json");
        assert!(DriveSyncConfig::validate(&cfg)
            .unwrap_err()
            .contains("must not be stored in Google Drive"));
    }
}
