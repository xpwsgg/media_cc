mod copier;
mod metadata;
mod scanner;

use copier::CopyResult;
use scanner::MediaFile;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use serde::Serialize;

/// Build the full PATH string with common binary locations appended.
/// macOS apps launched from Finder only see a minimal PATH (/usr/bin:/bin:/usr/sbin:/sbin),
/// so Homebrew and other common paths must be added explicitly.
pub fn get_full_path() -> String {
    let current = std::env::var("PATH").unwrap_or_default();

    let extra_dirs: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/opt/homebrew/bin",       // Apple Silicon Homebrew
            "/usr/local/bin",          // Intel Homebrew / manual installs
            "/opt/homebrew/sbin",
            "/usr/local/sbin",
        ]
    } else if cfg!(target_os = "linux") {
        &[
            "/usr/local/bin",
            "/usr/bin",
            "/snap/bin",
            "/home/linuxbrew/.linuxbrew/bin",
        ]
    } else {
        &[]
    };

    let mut parts: Vec<&str> = current.split(':').collect();
    for dir in extra_dirs {
        if !parts.contains(dir) {
            parts.push(dir);
        }
    }
    parts.join(":")
}

/// Create a Command for ffprobe with the enriched PATH.
pub fn ffprobe_command() -> Command {
    let mut cmd = Command::new("ffprobe");
    cmd.env("PATH", get_full_path());
    cmd
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemStatus {
    pub ffprobe_installed: bool,
    pub ffprobe_path: Option<String>,
    pub os_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub files: Vec<MediaFile>,
    pub total_count: usize,
    pub errors: Vec<scanner::ScanError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub current: usize,
    pub total: usize,
    pub current_file: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEvent {
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CopyCompleteEvent {
    pub copied: usize,
    pub skipped: usize,
    pub errors: usize,
    pub cancelled: bool,
}

pub struct AppState {
    pub cancel_flag: Arc<AtomicBool>,
    pub is_running: Arc<Mutex<bool>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            cancel_flag: Arc::new(AtomicBool::new(false)),
            is_running: Arc::new(Mutex::new(false)),
        }
    }
}

#[tauri::command]
async fn scan_source_directory(source_path: String) -> Result<ScanResult, String> {
    let path = PathBuf::from(&source_path);
    if !path.exists() {
        return Err("Source directory does not exist".to_string());
    }

    let scan_output = scanner::scan_directory(&path);
    let total_count = scan_output.files.len();

    Ok(ScanResult { files: scan_output.files, total_count, errors: scan_output.errors })
}

#[tauri::command]
async fn start_copy(
    app: AppHandle,
    state: State<'_, AppState>,
    source_path: String,
    dest_path: String,
) -> Result<(), String> {
    let mut is_running = state.is_running.lock().await;
    if *is_running {
        return Err("Copy operation already in progress".to_string());
    }
    *is_running = true;
    drop(is_running);

    // Reset cancel flag
    state.cancel_flag.store(false, Ordering::SeqCst);

    let cancel_flag = state.cancel_flag.clone();
    let is_running_flag = state.is_running.clone();

    // Run copy operation in background
    tokio::spawn(async move {
        let source = PathBuf::from(&source_path);
        let dest = PathBuf::from(&dest_path);

        // Emit log
        let _ = app.emit(
            "copy-log",
            LogEvent {
                level: "info".to_string(),
                message: format!("Starting scan of {}", source_path),
            },
        );

        // Scan files
        let scan_output = scanner::scan_directory(&source);
        let total = scan_output.files.len();
        let files = scan_output.files;

        // Log scan errors
        for scan_err in &scan_output.errors {
            let _ = app.emit(
                "copy-log",
                LogEvent {
                    level: "warn".to_string(),
                    message: format!("[扫描警告] {} - {}", scan_err.path, scan_err.message),
                },
            );
        }

        let _ = app.emit(
            "copy-log",
            LogEvent {
                level: "info".to_string(),
                message: format!("Found {} media files", total),
            },
        );

        let mut copied = 0;
        let mut skipped = 0;
        let mut errors = 0;

        for (index, file) in files.iter().enumerate() {
            // Check cancel flag
            if cancel_flag.load(Ordering::SeqCst) {
                let _ = app.emit(
                    "copy-log",
                    LogEvent {
                        level: "warn".to_string(),
                        message: "Operation cancelled by user".to_string(),
                    },
                );
                break;
            }

            let file_name = file
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            // Emit progress
            let _ = app.emit(
                "copy-progress",
                ProgressEvent {
                    current: index + 1,
                    total,
                    current_file: file_name.to_string(),
                    status: "processing".to_string(),
                },
            );

            // Log file type detection
            let file_type = if file.is_video { "视频" } else { "图片" };
            let _ = app.emit(
                "copy-log",
                LogEvent {
                    level: "info".to_string(),
                    message: format!("[检测] {} - 类型: {}", file_name, file_type),
                },
            );

            // Extract creation date and log the result
            let date_result = metadata::extract_creation_date(&file.path, file.is_video);

            let date_info = match &date_result {
                Some((d, _)) => format!("{}", d.format("%Y-%m-%d")),
                None => "未找到 (将使用 1970-01-01)".to_string(),
            };

            let source_info = match &date_result {
                Some((_, source)) => *source,
                None => if file.is_video { "ffprobe 未返回有效日期" } else { "EXIF 未找到日期信息" }
            };

            let _ = app.emit(
                "copy-log",
                LogEvent {
                    level: if date_result.is_some() { "info".to_string() } else { "warn".to_string() },
                    message: format!("[元数据] {} - 创建日期: {} (来源: {})", file_name, date_info, source_info),
                },
            );

            // Check file size and warn for large files
            if let Ok(file_meta) = std::fs::metadata(&file.path) {
                let size_mb = file_meta.len() / (1024 * 1024);
                if size_mb > 100 {
                    let _ = app.emit(
                        "copy-log",
                        LogEvent {
                            level: "info".to_string(),
                            message: format!("[提示] {} 文件较大 ({}MB)，正在计算校验和...", file_name, size_mb),
                        },
                    );
                }
            }

            // Copy file
            let date = date_result.map(|(d, _)| d);
            match copier::copy_file(&file.path, &dest, date) {
                Ok(CopyResult::Copied { dest: dest_file }) => {
                    copied += 1;
                    let _ = app.emit(
                        "copy-log",
                        LogEvent {
                            level: "success".to_string(),
                            message: format!(
                                "Copied: {} -> {}",
                                file_name,
                                dest_file.display()
                            ),
                        },
                    );
                }
                Ok(CopyResult::Skipped { reason }) => {
                    skipped += 1;
                    let _ = app.emit(
                        "copy-log",
                        LogEvent {
                            level: "info".to_string(),
                            message: format!("Skipped: {} ({})", file_name, reason),
                        },
                    );
                }
                Err(e) => {
                    errors += 1;
                    let _ = app.emit(
                        "copy-log",
                        LogEvent {
                            level: "error".to_string(),
                            message: format!("Error: {} - {}", file_name, e),
                        },
                    );
                }
            }
        }

        // Emit completion
        let _ = app.emit(
            "copy-complete",
            CopyCompleteEvent {
                copied,
                skipped,
                errors,
                cancelled: cancel_flag.load(Ordering::SeqCst),
            },
        );

        // Reset running flag
        let mut is_running = is_running_flag.lock().await;
        *is_running = false;
    });

    Ok(())
}

#[tauri::command]
async fn cancel_copy(state: State<'_, AppState>) -> Result<(), String> {
    state.cancel_flag.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
async fn is_copy_running(state: State<'_, AppState>) -> Result<bool, String> {
    let is_running = state.is_running.lock().await;
    Ok(*is_running)
}

#[tauri::command]
fn check_ffprobe() -> SystemStatus {
    // Detect OS type
    let os_type = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }.to_string();

    // Try to run ffprobe to check if it's installed
    // Use enriched PATH so bundled apps can find binaries outside /usr/bin
    let full_path = get_full_path();
    let output = ffprobe_command()
        .arg("-version")
        .output();

    match output {
        Ok(result) if result.status.success() => {
            // Try to get the path using 'which' on Unix or 'where' on Windows
            let path = if cfg!(target_os = "windows") {
                Command::new("where")
                    .arg("ffprobe")
                    .env("PATH", &full_path)
                    .output()
            } else {
                Command::new("which")
                    .arg("ffprobe")
                    .env("PATH", &full_path)
                    .output()
            }
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    String::from_utf8(o.stdout).ok().map(|s| s.lines().next().unwrap_or("").trim().to_string())
                } else {
                    None
                }
            });

            SystemStatus {
                ffprobe_installed: true,
                ffprobe_path: path,
                os_type,
            }
        }
        _ => SystemStatus {
            ffprobe_installed: false,
            ffprobe_path: None,
            os_type,
        },
    }
}

const ALLOWED_INSTALL_COMMANDS: &[&str] = &[
    "brew install ffmpeg",
    "winget install ffmpeg",
    "sudo apt install ffmpeg",
    "sudo dnf install ffmpeg",
    "sudo pacman -S ffmpeg",
];

fn is_allowed_install_command(command: &str) -> bool {
    ALLOWED_INSTALL_COMMANDS.contains(&command.trim())
}

fn escape_applescript_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[tauri::command]
fn open_terminal_with_command(command: String) -> Result<(), String> {
    if !is_allowed_install_command(&command) {
        return Err("Command not allowed: only predefined install commands are permitted".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("osascript")
            .args([
                "-e",
                &format!(
                    r#"tell application "Terminal"
                        activate
                        do script "{}"
                    end tell"#,
                    escape_applescript_string(&command)
                ),
            ])
            .spawn()
            .map_err(|e| format!("Failed to open terminal: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/c", "start", "cmd", "/k", &command])
            .spawn()
            .map_err(|e| format!("Failed to open terminal: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        let terminals = ["gnome-terminal", "konsole", "xfce4-terminal", "xterm"];
        let mut opened = false;

        for term in terminals {
            let result = match term {
                "gnome-terminal" => Command::new(term).args(["--", "bash", "-c", &format!("{}; exec bash", command)]).spawn(),
                "konsole" => Command::new(term).args(["-e", "bash", "-c", &format!("{}; exec bash", command)]).spawn(),
                "xfce4-terminal" => Command::new(term).args(["-e", &format!("bash -c '{}; exec bash'", command)]).spawn(),
                _ => Command::new(term).args(["-e", &format!("bash -c '{}; exec bash'", command)]).spawn(),
            };

            if result.is_ok() {
                opened = true;
                break;
            }
        }

        if !opened {
            return Err("No supported terminal emulator found".to_string());
        }
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            scan_source_directory,
            start_copy,
            cancel_copy,
            is_copy_running,
            check_ffprobe,
            open_terminal_with_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
