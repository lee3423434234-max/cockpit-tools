use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value as JsonValue};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

const APP_SERVER_RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);

#[cfg(target_os = "macos")]
const MACOS_APP_SERVER_CANDIDATES: &[&str] = &[
    "/Applications/ChatGPT.app/Contents/Resources/codex",
    "/Applications/Codex.app/Contents/Resources/codex",
];

/// Ask the official Codex app-server to rebuild its thread metadata after a
/// stopped-state session import.
pub fn rebuild_thread_metadata(
    codex_home: &Path,
    configured_executable: Option<&Path>,
) -> Result<(), String> {
    let executable = resolve_app_server_executable(configured_executable)?;
    let mut child = build_app_server_command(&executable, codex_home)
        .spawn()
        .map_err(|error| {
            format!(
                "failed to start Codex app-server ({} / CODEX_HOME={}): {}",
                executable.display(),
                codex_home.display(),
                error
            )
        })?;

    let stdout = child
        .stdout
        .take()
        .ok_or("failed to capture Codex app-server stdout")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("failed to capture Codex app-server stderr")?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or("failed to capture Codex app-server stdin")?;

    let (sender, receiver) = mpsc::channel::<String>();
    let stdout_reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = sender.send(line);
        }
    });
    let stderr_reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            tracing::warn!(target: "codex_app_server", "{}", line);
        }
    });

    let result = (|| {
        send_request(
            &mut stdin,
            json!({
                "method": "initialize",
                "id": 1,
                "params": {
                    "clientInfo": {
                        "name": "cockpit-tools",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": null,
                },
            }),
        )?;
        wait_for_response(&receiver, 1)?;

        send_request(
            &mut stdin,
            json!({
                "method": "thread/list",
                "id": 2,
                "params": {
                    "cursor": null,
                    "limit": 1,
                    "sortKey": "updated_at",
                    "sortDirection": "desc",
                    "modelProviders": null,
                    "sourceKinds": [],
                    "archived": false,
                },
            }),
        )?;
        wait_for_response(&receiver, 2)
    })();

    finish_child(&mut child);
    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
    result
}

pub fn resolve_app_server_executable(
    configured_executable: Option<&Path>,
) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("CODEX_APP_SERVER_EXECUTABLE") {
        push_candidate(&mut candidates, PathBuf::from(path));
    }
    if let Some(path) = configured_executable {
        push_candidate_from_launch_path(&mut candidates, path);
    }
    if let Some(path) = super::process::detect_codex_exec_path() {
        push_candidate_from_launch_path(&mut candidates, &path);
    }

    #[cfg(target_os = "macos")]
    for path in MACOS_APP_SERVER_CANDIDATES {
        push_candidate(&mut candidates, PathBuf::from(path));
    }

    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }

    Err(format!(
        "Codex app-server executable not found; searched: {}. Set CODEX_APP_SERVER_EXECUTABLE to override.",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn push_candidate_from_launch_path(candidates: &mut Vec<PathBuf>, launch_path: &Path) {
    if let Some(path) = app_server_from_launch_path(launch_path) {
        push_candidate(candidates, path);
    }
}

fn push_candidate(candidates: &mut Vec<PathBuf>, path: PathBuf) {
    if !path.as_os_str().is_empty() && !candidates.iter().any(|item| item == &path) {
        candidates.push(path);
    }
}

fn app_server_from_launch_path(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        return None;
    }
    if is_app_server_path(path) {
        return Some(path.to_path_buf());
    }
    if path_file_name_eq(path, "codex.app") || path_file_name_eq(path, "chatgpt.app") {
        return Some(path.join("Contents").join("Resources").join("codex"));
    }
    if (path_file_name_eq(path, "codex") || path_file_name_eq(path, "chatgpt"))
        && parent_file_name_eq(path, "macos")
    {
        return Some(path.parent()?.parent()?.join("Resources").join("codex"));
    }
    if path_file_name_eq(path, "codex.exe") || path_file_name_eq(path, "chatgpt.exe") {
        return Some(path.parent()?.join("resources").join("codex.exe"));
    }
    None
}

fn is_app_server_path(path: &Path) -> bool {
    (path_file_name_eq(path, "codex") && parent_file_name_eq(path, "resources"))
        || (path_file_name_eq(path, "codex.exe") && parent_file_name_eq(path, "resources"))
}

fn path_file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn parent_file_name_eq(path: &Path, expected: &str) -> bool {
    path.parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn build_app_server_command(executable: &Path, codex_home: &Path) -> Command {
    let mut command = Command::new(executable);
    super::process::apply_managed_proxy_env_to_command(&mut command);
    command
        .args(["app-server", "--listen", "stdio://"])
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn send_request(stdin: &mut impl Write, request: JsonValue) -> Result<(), String> {
    let line = serde_json::to_string(&request)
        .map_err(|error| format!("failed to serialize app-server request: {error}"))?;
    stdin
        .write_all(line.as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush())
        .map_err(|error| format!("failed to write app-server request: {error}"))
}

fn wait_for_response(receiver: &mpsc::Receiver<String>, request_id: i64) -> Result<(), String> {
    loop {
        let line = receiver
            .recv_timeout(APP_SERVER_RESPONSE_TIMEOUT)
            .map_err(|_| format!("timed out waiting for app-server response id={request_id}"))?;
        let Ok(value) = serde_json::from_str::<JsonValue>(&line) else {
            continue;
        };
        if value.get("id").and_then(JsonValue::as_i64) != Some(request_id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(format!(
                "Codex app-server returned an error for id={request_id}: {error}"
            ));
        }
        return value
            .get("result")
            .map(|_| ())
            .ok_or_else(|| format!("app-server response id={request_id} has no result"));
    }
}

fn finish_child(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_macos_app_to_resource_binary() {
        assert_eq!(
            app_server_from_launch_path(Path::new("/Applications/Codex.app")),
            Some(PathBuf::from(
                "/Applications/Codex.app/Contents/Resources/codex"
            ))
        );
        assert_eq!(
            app_server_from_launch_path(Path::new(
                "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT"
            )),
            Some(PathBuf::from(
                "/Applications/ChatGPT.app/Contents/Resources/codex"
            ))
        );
    }

    #[test]
    fn maps_windows_app_to_resource_binary() {
        assert_eq!(
            app_server_from_launch_path(Path::new("C:/Apps/Codex.exe")),
            Some(PathBuf::from("C:/Apps/resources/codex.exe"))
        );
    }
}
