use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};
use cockpit_core::modules::codex_drive_sync::{
    default_codex_home, default_state_path, DriveSyncConfig, PathMapping, SyncEngine,
};
use cockpit_core::modules::{cursor_account, github_copilot_account};
use colored::*;
use tabled::{Table, Tabled};

#[derive(Parser)]
#[command(author, version, about = "Cockpit Tools CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Codex session and account operations
    Codex {
        #[command(subcommand)]
        command: Box<CodexCommands>,
    },
    /// List accounts for a platform
    List {
        /// The platform (cursor, copilot)
        platform: String,
    },
    /// Switch accounts for a specific platform
    Switch {
        /// The platform (cursor, copilot)
        platform: String,
        /// The account ID or email to switch to
        account: String,
    },
    /// Show current quota for a platform
    Quota {
        /// The platform (cursor, copilot)
        platform: String,
    },
}

#[derive(Subcommand)]
enum CodexCommands {
    /// Manage encrypted cross-device Codex session snapshots
    Sessions {
        #[command(subcommand)]
        command: SessionCommands,
    },
}

#[derive(Subcommand)]
enum SessionCommands {
    /// Inspect local and Google Drive synchronization state
    Status(SyncArgs),
    /// Upload and reconcile all sessions once
    SyncOnce(SyncArgs),
    /// Reconcile repeatedly until Ctrl+C is pressed
    Daemon {
        #[command(flatten)]
        sync: SyncArgs,
        /// Polling interval in seconds
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(10..))]
        interval_seconds: u64,
    },
    /// Accept one encrypted object as the conflict winner
    ResolveConflict {
        #[command(flatten)]
        sync: SyncArgs,
        /// Canonical SHA-256 object identifier to accept
        #[arg(long)]
        object_sha: String,
    },
}

#[derive(Debug, Clone, Args)]
struct SyncArgs {
    /// Codex data directory (defaults to CODEX_HOME or ~/.codex)
    #[arg(long, env = "CODEX_HOME")]
    codex_home: Option<PathBuf>,
    /// Google Drive transport directory, for example G:\\My Drive\\Codex Sessions AutoSync
    #[arg(long, env = "COCKPIT_DRIVE_SYNC_ROOT")]
    drive_root: PathBuf,
    /// Local-only state path; never place this inside Google Drive
    #[arg(long, env = "COCKPIT_DRIVE_SYNC_STATE")]
    state_path: Option<PathBuf>,
    /// Stable device identifier; generated and persisted when omitted
    #[arg(long, env = "COCKPIT_DRIVE_SYNC_DEVICE_ID")]
    device_id: Option<String>,
    /// Name of the environment variable containing the encryption passphrase
    #[arg(
        long,
        default_value = "COCKPIT_DRIVE_SYNC_PASSPHRASE",
        value_name = "ENV_VAR"
    )]
    passphrase_env: String,
    /// Rewrite a source cwd prefix during import; may be repeated
    #[arg(long = "map-cwd", value_parser = parse_path_mapping, value_name = "FROM=TO")]
    cwd_mappings: Vec<PathMapping>,
    /// Override session_meta.model_provider during import
    #[arg(long)]
    provider: Option<String>,
    /// Explicit Codex app-server executable for metadata rebuild
    #[arg(long, env = "CODEX_APP_SERVER_EXECUTABLE")]
    app_server_executable: Option<PathBuf>,
    /// Show intended changes without writing local or Drive files
    #[arg(long)]
    dry_run: bool,
    /// Publish encrypted local snapshots without importing remote sessions
    #[arg(long)]
    upload_only: bool,
    /// Skip official app-server metadata rebuild (intended for tests/recovery only)
    #[arg(long)]
    skip_index_rebuild: bool,
}

#[derive(Tabled)]
struct AccountDisplay {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Email")]
    email: String,
    #[tabled(rename = "Plan")]
    plan: String,
    #[tabled(rename = "Tags")]
    tags: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Codex { command }) => run_codex_command(*command).await?,
        Some(Commands::List { platform }) => match platform.to_lowercase().as_str() {
            "cursor" => {
                let accounts = cursor_account::list_accounts();
                display_accounts(
                    accounts
                        .iter()
                        .map(|a| AccountDisplay {
                            id: a.id.clone(),
                            email: a.email.clone(),
                            plan: a.membership_type.clone().unwrap_or_default(),
                            tags: a.tags.as_ref().map(|t| t.join(", ")).unwrap_or_default(),
                        })
                        .collect(),
                );
            }
            "copilot" | "github_copilot" => {
                let accounts = github_copilot_account::list_accounts();
                display_accounts(
                    accounts
                        .iter()
                        .map(|a| AccountDisplay {
                            id: a.id.clone(),
                            email: a.github_email.clone().unwrap_or_default(),
                            plan: a.copilot_plan.clone().unwrap_or_default(),
                            tags: a.tags.as_ref().map(|t| t.join(", ")).unwrap_or_default(),
                        })
                        .collect(),
                );
            }
            _ => println!("{} Unknown platform: {}", "Error:".red(), platform),
        },
        Some(Commands::Switch { platform, account }) => match platform.to_lowercase().as_str() {
            "cursor" => {
                if let Err(e) = cursor_account::inject_to_cursor(&account) {
                    println!("{} {}", "Error:".red(), e);
                } else {
                    println!(
                        "{} Successfully switched Cursor account to {}",
                        "Success:".green(),
                        account
                    );
                }
            }
            "copilot" | "github_copilot" => {
                println!("{} GitHub Copilot switch is partially implemented in CLI. Use GUI for full instance sync.", "Info:".yellow());
            }
            _ => println!("{} Unknown platform: {}", "Error:".red(), platform),
        },
        Some(Commands::Quota { platform }) => match platform.to_lowercase().as_str() {
            _ => println!(
                "{} Quota command not yet implemented for {}",
                "Info:".yellow(),
                platform
            ),
        },
        None => {
            println!("Welcome to Cockpit CLI! Use --help for commands.");
        }
    }

    Ok(())
}

async fn run_codex_command(command: CodexCommands) -> anyhow::Result<()> {
    match command {
        CodexCommands::Sessions { command } => match command {
            SessionCommands::Status(args) => {
                let engine = SyncEngine::new(build_sync_config(&args, false)?)
                    .map_err(anyhow::Error::msg)?;
                let status = engine.status().map_err(anyhow::Error::msg)?;
                println!("{}", serde_json::to_string_pretty(&status)?);
            }
            SessionCommands::SyncOnce(args) => {
                let engine =
                    SyncEngine::new(build_sync_config(&args, true)?).map_err(anyhow::Error::msg)?;
                let summary = engine.sync_once().map_err(anyhow::Error::msg)?;
                println!("{}", serde_json::to_string_pretty(&summary)?);
                if summary.index_rebuild_pending {
                    anyhow::bail!(
                        "session files synchronized, but Codex metadata rebuild remains pending"
                    );
                }
            }
            SessionCommands::Daemon {
                sync,
                interval_seconds,
            } => {
                let engine =
                    SyncEngine::new(build_sync_config(&sync, true)?).map_err(anyhow::Error::msg)?;
                println!(
                    "{}",
                    format!(
                        "Codex session sync daemon polling every {interval_seconds} seconds; press Ctrl+C to stop"
                    )
                    .cyan()
                );
                loop {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => {
                            println!("{}", "Stopping sync daemon.".yellow());
                            break;
                        }
                        _ = async {
                            match engine.sync_once() {
                                Ok(summary) => match serde_json::to_string(&summary) {
                                    Ok(json) => println!("{json}"),
                                    Err(error) => eprintln!("{} {error}", "Error:".red()),
                                },
                                Err(error) => eprintln!("{} {error}", "Sync deferred:".yellow()),
                            }
                            tokio::time::sleep(Duration::from_secs(interval_seconds)).await;
                        } => {}
                    }
                }
            }
            SessionCommands::ResolveConflict { sync, object_sha } => {
                let engine =
                    SyncEngine::new(build_sync_config(&sync, true)?).map_err(anyhow::Error::msg)?;
                let summary = engine
                    .resolve_conflict(&object_sha)
                    .map_err(anyhow::Error::msg)?;
                println!("{}", serde_json::to_string_pretty(&summary)?);
            }
        },
    }
    Ok(())
}

fn build_sync_config(args: &SyncArgs, require_passphrase: bool) -> anyhow::Result<DriveSyncConfig> {
    let passphrase = match std::env::var(&args.passphrase_env) {
        Ok(value) => value,
        Err(_) if require_passphrase => anyhow::bail!(
            "environment variable {} must contain the sync encryption passphrase",
            args.passphrase_env
        ),
        Err(_) => String::new(),
    };
    Ok(DriveSyncConfig {
        codex_home: match &args.codex_home {
            Some(path) => path.clone(),
            None => default_codex_home().map_err(anyhow::Error::msg)?,
        },
        drive_root: args.drive_root.clone(),
        state_path: match &args.state_path {
            Some(path) => path.clone(),
            None => default_state_path().map_err(anyhow::Error::msg)?,
        },
        passphrase,
        device_id: args.device_id.clone(),
        cwd_mappings: args.cwd_mappings.clone(),
        provider_override: args.provider.clone(),
        app_server_executable: args.app_server_executable.clone(),
        dry_run: args.dry_run,
        upload_only: args.upload_only,
        skip_index_rebuild: args.skip_index_rebuild,
    })
}

fn parse_path_mapping(value: &str) -> Result<PathMapping, String> {
    PathMapping::parse(value)
}

fn display_accounts(accounts: Vec<AccountDisplay>) {
    if accounts.is_empty() {
        println!("No accounts found.");
    } else {
        println!("{}", Table::new(accounts).to_string());
    }
}
