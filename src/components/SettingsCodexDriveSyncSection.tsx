import { useCallback, useMemo, useRef, useState, useEffect } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import {
  AlertTriangle,
  ArrowLeftRight,
  CheckCircle2,
  Cloud,
  Database,
  Eye,
  EyeOff,
  FolderOpen,
  GitBranch,
  KeyRound,
  LockKeyhole,
  RefreshCw,
  Search,
  ShieldCheck,
  Trash2,
  Upload,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  CodexDriveSyncConflict,
  CodexDriveSyncStatus,
  CodexDriveSyncSummary,
  CodexDriveSyncResolveSummary,
  getCodexDriveSyncStatus,
  listCodexDriveSyncConflicts,
  resolveCodexDriveSyncConflict,
  runCodexDriveSync,
} from '../services/codexDriveSyncService';
import './SettingsCodexDriveSyncSection.css';

const DRIVE_ROOT_STORAGE_KEY = 'cockpit.codexDriveSync.driveRoot';
const MAPPINGS_STORAGE_KEY = 'cockpit.codexDriveSync.cwdMappings';
const PROVIDER_STORAGE_KEY = 'cockpit.codexDriveSync.providerOverride';

type BusyAction =
  | 'status'
  | 'upload-preview'
  | 'upload'
  | 'sync-preview'
  | 'sync'
  | 'resolve-preview'
  | 'resolve';

type FeedbackTone = 'loading' | 'success' | 'error';

interface Feedback {
  tone: FeedbackTone;
  text: string;
}

interface OverviewResult {
  status: CodexDriveSyncStatus;
  conflicts: CodexDriveSyncConflict[];
}

function readStoredValue(key: string): string {
  try {
    return window.localStorage.getItem(key) ?? '';
  } catch {
    return '';
  }
}

function storeValue(key: string, value: string) {
  try {
    if (value) {
      window.localStorage.setItem(key, value);
    } else {
      window.localStorage.removeItem(key);
    }
  } catch {
    // The settings remain usable for this window when storage is unavailable.
  }
}

function normalizeError(error: unknown): string {
  return String(error).replace(/^Error:\s*/, '');
}

function shortHash(value: string | null | undefined): string {
  if (!value) return '—';
  return value.length > 14 ? `${value.slice(0, 12)}…` : value;
}

function formatDetectedAt(value: string): string {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

async function loadOverview(driveRoot: string): Promise<OverviewResult> {
  const [status, conflicts] = await Promise.all([
    getCodexDriveSyncStatus(driveRoot),
    listCodexDriveSyncConflicts(driveRoot),
  ]);
  return { status, conflicts };
}

export function SettingsCodexDriveSyncSection() {
  const { t } = useTranslation();
  const autoLoadAttempted = useRef(false);
  const [driveRoot, setDriveRoot] = useState(() => readStoredValue(DRIVE_ROOT_STORAGE_KEY));
  const [passphrase, setPassphrase] = useState('');
  const [showPassphrase, setShowPassphrase] = useState(false);
  const [cwdMappings, setCwdMappings] = useState(() => readStoredValue(MAPPINGS_STORAGE_KEY));
  const [providerOverride, setProviderOverride] = useState(() => readStoredValue(PROVIDER_STORAGE_KEY));
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [status, setStatus] = useState<CodexDriveSyncStatus | null>(null);
  const [conflicts, setConflicts] = useState<CodexDriveSyncConflict[]>([]);
  const [summary, setSummary] = useState<CodexDriveSyncSummary | null>(null);
  const [resolveSummary, setResolveSummary] = useState<CodexDriveSyncResolveSummary | null>(null);
  const [busyAction, setBusyAction] = useState<BusyAction | null>(null);
  const [feedback, setFeedback] = useState<Feedback | null>(null);

  const busy = busyAction !== null;
  const mappingLines = useMemo(
    () => cwdMappings.split(/\r?\n/).map((value) => value.trim()).filter(Boolean),
    [cwdMappings],
  );

  const applyOverview = useCallback((next: OverviewResult) => {
    setStatus(next.status);
    setConflicts(next.conflicts);
  }, []);

  const refreshOverview = useCallback(async (quiet = false) => {
    const root = driveRoot.trim();
    if (!root) {
      if (!quiet) {
        setFeedback({
          tone: 'error',
          text: t('settings.codexDriveSync.errors.driveRootRequired', {
            defaultValue: '請先選擇 Google Drive 同步資料夾。',
          }),
        });
      }
      return;
    }

    setBusyAction('status');
    if (!quiet) {
      setFeedback({
        tone: 'loading',
        text: t('settings.codexDriveSync.feedback.scanning', {
          defaultValue: '正在掃描本機與 Google Drive 會話，較大的 rollout 可能需要一段時間…',
        }),
      });
    }
    try {
      applyOverview(await loadOverview(root));
      if (!quiet) {
        setFeedback({
          tone: 'success',
          text: t('settings.codexDriveSync.feedback.scanComplete', {
            defaultValue: '同步狀態已更新。',
          }),
        });
      }
    } catch (error) {
      setFeedback({
        tone: 'error',
        text: t('settings.codexDriveSync.feedback.scanFailed', {
          error: normalizeError(error),
          defaultValue: '讀取同步狀態失敗：{{error}}',
        }),
      });
    } finally {
      setBusyAction(null);
    }
  }, [applyOverview, driveRoot, t]);

  useEffect(() => {
    if (autoLoadAttempted.current) return;
    autoLoadAttempted.current = true;
    if (driveRoot.trim()) {
      void refreshOverview(true);
    }
  }, [driveRoot, refreshOverview]);

  const updateDriveRoot = useCallback((value: string) => {
    setDriveRoot(value);
    storeValue(DRIVE_ROOT_STORAGE_KEY, value.trim());
    setStatus(null);
    setConflicts([]);
    setSummary(null);
    setResolveSummary(null);
  }, []);

  const handlePickDriveRoot = useCallback(async () => {
    try {
      const selected = await open({ directory: true, multiple: false });
      if (typeof selected === 'string') {
        updateDriveRoot(selected);
      }
    } catch (error) {
      setFeedback({ tone: 'error', text: normalizeError(error) });
    }
  }, [updateDriveRoot]);

  const validateRunInputs = useCallback((): boolean => {
    if (!driveRoot.trim()) {
      setFeedback({
        tone: 'error',
        text: t('settings.codexDriveSync.errors.driveRootRequired', {
          defaultValue: '請先選擇 Google Drive 同步資料夾。',
        }),
      });
      return false;
    }
    if (passphrase.length < 12) {
      setFeedback({
        tone: 'error',
        text: t('settings.codexDriveSync.errors.passphraseTooShort', {
          defaultValue: '同步密碼至少需要 12 個字元；密碼只保留在目前視窗記憶體中。',
        }),
      });
      return false;
    }
    return true;
  }, [driveRoot, passphrase.length, t]);

  const performSync = useCallback(async (dryRun: boolean, uploadOnly: boolean) => {
    if (!validateRunInputs()) return;
    if (!dryRun && status?.codex_running) {
      setFeedback({
        tone: 'error',
        text: t('settings.codexDriveSync.errors.codexRunning', {
          defaultValue: 'Codex 正在運行。請關閉所有 Codex 視窗後再執行實際同步。',
        }),
      });
      return;
    }
    if (!dryRun) {
      const confirmed = window.confirm(
        uploadOnly
          ? t('settings.codexDriveSync.confirm.upload', {
              defaultValue: '確定要將本機會話加密後上傳到 Google Drive？這不會匯入遠端內容。',
            })
          : t('settings.codexDriveSync.confirm.sync', {
              defaultValue: '確定要執行雙向同步？本機檔案可能在備份後被更新，分岔會保留為衝突。',
            }),
      );
      if (!confirmed) return;
    }

    const action: BusyAction = uploadOnly
      ? (dryRun ? 'upload-preview' : 'upload')
      : (dryRun ? 'sync-preview' : 'sync');
    setBusyAction(action);
    setSummary(null);
    setResolveSummary(null);
    setFeedback({
      tone: 'loading',
      text: dryRun
        ? t('settings.codexDriveSync.feedback.previewing', {
            defaultValue: '正在計算預演結果；不會寫入本機或 Google Drive…',
          })
        : t('settings.codexDriveSync.feedback.syncing', {
            defaultValue: '正在加密並同步會話，請保持 Cockpit Tools 開啟…',
          }),
    });
    try {
      const result = await runCodexDriveSync({
        driveRoot: driveRoot.trim(),
        passphrase,
        cwdMappings: mappingLines,
        providerOverride: providerOverride.trim() || null,
        dryRun,
        uploadOnly,
      });
      setSummary(result);
      setFeedback({
        tone: result.warnings.length > 0 ? 'error' : 'success',
        text: dryRun
          ? t('settings.codexDriveSync.feedback.previewComplete', {
              defaultValue: '安全預演完成，尚未寫入任何檔案。',
            })
          : t('settings.codexDriveSync.feedback.syncComplete', {
              defaultValue: '同步完成。請核對下方摘要與 Google Drive 物件數量。',
            }),
      });
      if (!dryRun) {
        setPassphrase('');
        applyOverview(await loadOverview(driveRoot.trim()));
      }
    } catch (error) {
      setFeedback({
        tone: 'error',
        text: t('settings.codexDriveSync.feedback.syncFailed', {
          error: normalizeError(error),
          defaultValue: '同步失敗：{{error}}',
        }),
      });
    } finally {
      setBusyAction(null);
    }
  }, [applyOverview, driveRoot, mappingLines, passphrase, providerOverride, status?.codex_running, t, validateRunInputs]);

  const handleResolve = useCallback(async (objectSha256: string, dryRun: boolean) => {
    if (!validateRunInputs()) return;
    if (!dryRun && status?.codex_running) {
      setFeedback({
        tone: 'error',
        text: t('settings.codexDriveSync.errors.codexRunning', {
          defaultValue: 'Codex 正在運行。請關閉所有 Codex 視窗後再套用衝突版本。',
        }),
      });
      return;
    }
    if (!dryRun && !window.confirm(t('settings.codexDriveSync.confirm.resolve', {
      hash: shortHash(objectSha256),
      defaultValue: '確定使用遠端物件 {{hash}}？本機 rollout 與索引會先備份，再套用選取版本。',
    }))) {
      return;
    }

    setBusyAction(dryRun ? 'resolve-preview' : 'resolve');
    setResolveSummary(null);
    setFeedback({
      tone: 'loading',
      text: dryRun
        ? t('settings.codexDriveSync.feedback.resolvePreviewing', {
            defaultValue: '正在預演衝突套用…',
          })
        : t('settings.codexDriveSync.feedback.resolving', {
            defaultValue: '正在備份並套用選取的遠端版本…',
          }),
    });
    try {
      const result = await resolveCodexDriveSyncConflict({
        driveRoot: driveRoot.trim(),
        passphrase,
        cwdMappings: mappingLines,
        providerOverride: providerOverride.trim() || null,
        objectSha256,
        dryRun,
      });
      setResolveSummary(result);
      setFeedback({
        tone: 'success',
        text: dryRun
          ? t('settings.codexDriveSync.feedback.resolvePreviewComplete', {
              defaultValue: '衝突套用預演完成，尚未修改檔案。',
            })
          : t('settings.codexDriveSync.feedback.resolveComplete', {
              defaultValue: '已備份本機資料並套用選取版本。',
            }),
      });
      if (!dryRun) {
        setPassphrase('');
        applyOverview(await loadOverview(driveRoot.trim()));
      }
    } catch (error) {
      setFeedback({
        tone: 'error',
        text: t('settings.codexDriveSync.feedback.resolveFailed', {
          error: normalizeError(error),
          defaultValue: '衝突處理失敗：{{error}}',
        }),
      });
    } finally {
      setBusyAction(null);
    }
  }, [applyOverview, driveRoot, mappingLines, passphrase, providerOverride, status?.codex_running, t, validateRunInputs]);

  const statusCards = status ? [
    { label: t('settings.codexDriveSync.stats.local', { defaultValue: '本機會話' }), value: status.local_sessions, icon: Database },
    { label: t('settings.codexDriveSync.stats.objects', { defaultValue: '加密物件' }), value: status.remote_objects, icon: LockKeyhole },
    { label: t('settings.codexDriveSync.stats.heads', { defaultValue: '裝置 Heads' }), value: status.remote_heads, icon: GitBranch },
    { label: t('settings.codexDriveSync.stats.conflicts', { defaultValue: '待處理衝突' }), value: status.conflicts, icon: AlertTriangle },
  ] : [];

  const summaryCards = summary ? [
    [t('settings.codexDriveSync.result.local', { defaultValue: '掃描會話' }), summary.local_sessions],
    [t('settings.codexDriveSync.result.uploaded', { defaultValue: summary.dry_run ? '預計上傳' : '已上傳' }), summary.uploaded_objects],
    [t('settings.codexDriveSync.result.heads', { defaultValue: summary.dry_run ? '預計 Heads' : '已發佈 Heads' }), summary.published_heads],
    [t('settings.codexDriveSync.result.imported', { defaultValue: '匯入' }), summary.imported_sessions],
    [t('settings.codexDriveSync.result.fastForwarded', { defaultValue: '快進更新' }), summary.fast_forwarded_sessions],
    [t('settings.codexDriveSync.result.noop', { defaultValue: '無需變更' }), summary.no_op_sessions],
  ] : [];

  return (
    <section className="codex-drive-sync-section" aria-labelledby="codex-drive-sync-title">
      <div className="group-title codex-drive-sync-heading" id="codex-drive-sync-title">
        <Cloud size={18} />
        {t('settings.codexDriveSync.groupTitle', { defaultValue: 'Codex Google Drive 會話同步' })}
      </div>
      <div className="settings-group codex-drive-sync-card">
        <div className="codex-drive-sync-safety">
          <ShieldCheck size={21} />
          <div>
            <strong>{t('settings.codexDriveSync.safetyTitle', { defaultValue: '端到端加密傳輸' })}</strong>
            <p>{t('settings.codexDriveSync.safetyDesc', {
              defaultValue: '只同步 sessions 與 archived_sessions 的 rollout。密碼不會保存；auth.json、設定、日誌與 SQLite 永遠不會上傳。',
            })}</p>
          </div>
        </div>

        <div className="codex-drive-sync-form-grid">
          <label className="codex-drive-sync-field codex-drive-sync-field--wide">
            <span>{t('settings.codexDriveSync.driveRootTitle', { defaultValue: 'Google Drive 本機資料夾' })}</span>
            <small>{t('settings.codexDriveSync.driveRootDesc', { defaultValue: 'Windows 例如 G:\\My Drive\\Codex Sessions AutoSync；macOS 請選擇 CloudStorage 下的資料夾。' })}</small>
            <div className="codex-drive-sync-input-row">
              <input
                className="settings-input settings-input--path"
                value={driveRoot}
                placeholder={t('settings.codexDriveSync.driveRootPlaceholder', { defaultValue: '選擇或輸入同步資料夾' })}
                disabled={busy}
                onChange={(event) => updateDriveRoot(event.target.value)}
              />
              <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void handlePickDriveRoot()}>
                <FolderOpen size={16} />
                {t('common.select', { defaultValue: '選擇' })}
              </button>
            </div>
          </label>

          <label className="codex-drive-sync-field codex-drive-sync-field--wide">
            <span>{t('settings.codexDriveSync.passphraseTitle', { defaultValue: '共享加密密碼' })}</span>
            <small>{t('settings.codexDriveSync.passphraseDesc', { defaultValue: '至少 12 個字元，只保留在目前視窗記憶體；所有電腦必須使用相同密碼。' })}</small>
            <div className="codex-drive-sync-input-row">
              <div className="codex-drive-sync-password-wrap">
                <KeyRound size={16} />
                <input
                  type={showPassphrase ? 'text' : 'password'}
                  value={passphrase}
                  autoComplete="new-password"
                  spellCheck={false}
                  placeholder={t('settings.codexDriveSync.passphrasePlaceholder', { defaultValue: '不會保存密碼' })}
                  disabled={busy}
                  onChange={(event) => setPassphrase(event.target.value)}
                />
                <button
                  type="button"
                  className="codex-drive-sync-icon-button"
                  aria-label={showPassphrase ? t('common.hide', { defaultValue: '隱藏' }) : t('common.show', { defaultValue: '顯示' })}
                  onClick={() => setShowPassphrase((value) => !value)}
                >
                  {showPassphrase ? <EyeOff size={16} /> : <Eye size={16} />}
                </button>
              </div>
              <button type="button" className="btn btn-secondary" disabled={busy || !passphrase} onClick={() => setPassphrase('')}>
                <Trash2 size={16} />
                {t('common.clear', { defaultValue: '清除' })}
              </button>
            </div>
          </label>
        </div>

        <button
          type="button"
          className="codex-drive-sync-advanced-toggle"
          onClick={() => setShowAdvanced((value) => !value)}
          aria-expanded={showAdvanced}
        >
          <ArrowLeftRight size={16} />
          {showAdvanced
            ? t('settings.codexDriveSync.hideAdvanced', { defaultValue: '隱藏跨電腦對應' })
            : t('settings.codexDriveSync.showAdvanced', { defaultValue: '跨電腦路徑與 Provider 對應' })}
        </button>

        {showAdvanced ? (
          <div className="codex-drive-sync-advanced-grid">
            <label className="codex-drive-sync-field">
              <span>{t('settings.codexDriveSync.mappingTitle', { defaultValue: 'CWD 路徑對應' })}</span>
              <small>{t('settings.codexDriveSync.mappingDesc', { defaultValue: '每行一組 FROM=TO，例如 C:\\Users\\Tom\\Documents=/Users/tom/Documents' })}</small>
              <textarea
                value={cwdMappings}
                disabled={busy}
                rows={3}
                onChange={(event) => {
                  setCwdMappings(event.target.value);
                  storeValue(MAPPINGS_STORAGE_KEY, event.target.value);
                }}
              />
            </label>
            <label className="codex-drive-sync-field">
              <span>{t('settings.codexDriveSync.providerTitle', { defaultValue: '匯入 Provider 覆寫（選填）' })}</span>
              <small>{t('settings.codexDriveSync.providerDesc', { defaultValue: '只在匯入時重寫 session_meta.model_provider，例如 openai。' })}</small>
              <input
                value={providerOverride}
                disabled={busy}
                placeholder="openai"
                onChange={(event) => {
                  setProviderOverride(event.target.value);
                  storeValue(PROVIDER_STORAGE_KEY, event.target.value.trim());
                }}
              />
            </label>
          </div>
        ) : null}

        <div className="codex-drive-sync-toolbar">
          <div className="codex-drive-sync-toolbar-copy">
            <strong>{t('settings.codexDriveSync.actionsTitle', { defaultValue: '安全同步控制' })}</strong>
            <span>{status?.codex_running
              ? t('settings.codexDriveSync.codexRunning', { defaultValue: 'Codex 正在運行：只允許預演與查看狀態。' })
              : t('settings.codexDriveSync.codexStopped', { defaultValue: '未偵測到 Codex 程序，可執行實際同步。' })}</span>
          </div>
          <div className="codex-drive-sync-actions">
            <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void refreshOverview()}>
              <RefreshCw size={16} className={busyAction === 'status' ? 'loading-spinner' : undefined} />
              {t('common.refresh', { defaultValue: '重新整理' })}
            </button>
            <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void performSync(true, true)}>
              <Search size={16} />
              {t('settings.codexDriveSync.uploadPreview', { defaultValue: '上傳預演' })}
            </button>
            <button type="button" className="btn btn-primary" disabled={busy || status?.codex_running === true} onClick={() => void performSync(false, true)}>
              <Upload size={16} />
              {t('settings.codexDriveSync.uploadNow', { defaultValue: '只上傳' })}
            </button>
            <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void performSync(true, false)}>
              <Search size={16} />
              {t('settings.codexDriveSync.syncPreview', { defaultValue: '雙向預演' })}
            </button>
            <button type="button" className="btn btn-danger" disabled={busy || status?.codex_running === true} onClick={() => void performSync(false, false)}>
              <ArrowLeftRight size={16} />
              {t('settings.codexDriveSync.syncNow', { defaultValue: '雙向同步' })}
            </button>
          </div>
        </div>

        {feedback ? (
          <div className={`codex-drive-sync-feedback ${feedback.tone}`} role="status" aria-live="polite">
            {feedback.tone === 'loading' ? <RefreshCw size={17} className="loading-spinner" /> : feedback.tone === 'success' ? <CheckCircle2 size={17} /> : <AlertTriangle size={17} />}
            <span>{feedback.text}</span>
          </div>
        ) : null}

        {status ? (
          <div className="codex-drive-sync-overview">
            <div className="codex-drive-sync-stat-grid">
              {statusCards.map(({ label, value, icon: Icon }) => (
                <div className="codex-drive-sync-stat" key={label}>
                  <Icon size={17} />
                  <div><strong>{value.toLocaleString()}</strong><span>{label}</span></div>
                </div>
              ))}
            </div>
            <div className="codex-drive-sync-meta">
              <span>{t('settings.codexDriveSync.deviceId', { defaultValue: '裝置 ID' })}: <code>{status.device_id ?? t('settings.codexDriveSync.notCreated', { defaultValue: '尚未建立' })}</code></span>
              <span>{t('settings.codexDriveSync.partialFiles', { defaultValue: '忽略的 partial 檔案' })}: {status.partial_files}</span>
              <span>{t('settings.codexDriveSync.indexPending', { defaultValue: '索引重建待處理' })}: {status.index_rebuild_pending ? t('common.yes', { defaultValue: '是' }) : t('common.no', { defaultValue: '否' })}</span>
            </div>
          </div>
        ) : (
          <div className="codex-drive-sync-empty">
            <Cloud size={22} />
            <span>{t('settings.codexDriveSync.emptyStatus', { defaultValue: '選擇資料夾後重新整理，即可查看本機與雲端狀態。' })}</span>
          </div>
        )}

        {summary ? (
          <div className="codex-drive-sync-result">
            <div className="codex-drive-sync-result-title">
              {summary.dry_run ? t('settings.codexDriveSync.previewResultTitle', { defaultValue: '預演摘要' }) : t('settings.codexDriveSync.syncResultTitle', { defaultValue: '同步摘要' })}
            </div>
            <div className="codex-drive-sync-result-grid">
              {summaryCards.map(([label, value]) => <div key={String(label)}><span>{label}</span><strong>{value}</strong></div>)}
            </div>
            {summary.warnings.length > 0 ? (
              <ul className="codex-drive-sync-warnings">{summary.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>
            ) : null}
          </div>
        ) : null}

        {resolveSummary ? (
          <div className="codex-drive-sync-result">
            <div className="codex-drive-sync-result-title">{resolveSummary.dry_run ? t('settings.codexDriveSync.resolvePreviewTitle', { defaultValue: '衝突套用預演' }) : t('settings.codexDriveSync.resolveResultTitle', { defaultValue: '衝突已處理' })}</div>
            <div className="codex-drive-sync-resolution-path"><code>{resolveSummary.target_path}</code></div>
          </div>
        ) : null}

        {conflicts.length > 0 ? (
          <div className="codex-drive-sync-conflicts">
            <div className="codex-drive-sync-conflicts-title"><AlertTriangle size={18} />{t('settings.codexDriveSync.conflictsTitle', { count: conflicts.length, defaultValue: '待處理衝突（{{count}}）' })}</div>
            <p>{t('settings.codexDriveSync.conflictsDesc', { defaultValue: '系統不會自動合併分岔會話。請先預演，再手動選擇一個遠端加密版本。' })}</p>
            {conflicts.map((conflict) => (
              <article className="codex-drive-sync-conflict" key={`${conflict.session_key}-${conflict.detected_at}`}>
                <div className="codex-drive-sync-conflict-head"><code>{shortHash(conflict.session_key)}</code><span>{formatDetectedAt(conflict.detected_at)}</span></div>
                <div className="codex-drive-sync-conflict-local">{t('settings.codexDriveSync.localVersion', { defaultValue: '本機版本' })}: <code>{shortHash(conflict.local_content_sha256)}</code></div>
                <div className="codex-drive-sync-remote-list">
                  {conflict.remote_content_sha256.map((hash) => (
                    <div className="codex-drive-sync-remote" key={hash}>
                      <code title={hash}>{shortHash(hash)}</code>
                      <div>
                        <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void handleResolve(hash, true)}>{t('settings.codexDriveSync.previewResolve', { defaultValue: '預演套用' })}</button>
                        <button type="button" className="btn btn-danger" disabled={busy || status?.codex_running === true} onClick={() => void handleResolve(hash, false)}>{t('settings.codexDriveSync.useVersion', { defaultValue: '使用此版本' })}</button>
                      </div>
                    </div>
                  ))}
                </div>
              </article>
            ))}
          </div>
        ) : null}
      </div>
    </section>
  );
}
