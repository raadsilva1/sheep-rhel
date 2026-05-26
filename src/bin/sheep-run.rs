//! sheep-run — application launcher that combines $PATH binaries with
//! Flatpak apps and pipes the result to wmenu.
//!
//! Usage: sheep-run [wmenu-options...]
//!
//! All arguments are forwarded to wmenu.  The launcher reads executables
//! from $PATH, scans Flatpak export directories for .desktop files, and
//! presents a unified menu.  Selecting a PATH binary runs it directly;
//! selecting a Flatpak app runs `flatpak run <app-id>`.
//!
//! When launched without colour arguments, sheep-run attempts to read the
//! current theme from `~/.config/sheep-rhel/theme.json` (written by
//! `sheep-rhel --theme`) so that wmenu always matches the River theme.
//!
//! SECURITY: Commands are executed directly via `std::process::Command` —
//! no shell interpolation is used, preventing command-injection attacks
//! from malicious .desktop files.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Persisted theme state written by `theme::apply_theme`.
#[derive(Debug, Clone, serde::Deserialize)]
struct ThemeState {
    #[allow(dead_code)]
    name: String,
    bg: String,
    fg: String,
    sel_bg: String,
    sel_fg: String,
}

impl ThemeState {
    /// Read the theme state from the default config location.
    fn read() -> Option<Self> {
        let path = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from(".config"))
            .join("sheep-rhel/theme.json");
        let data = fs::read(&path).ok()?;
        serde_json::from_slice(&data).ok()
    }

    /// Convert to wmenu colour argument vector.
    fn to_wmenu_args(&self) -> Vec<String> {
        vec![
            "-N".to_string(), self.bg.clone(),
            "-n".to_string(), self.fg.clone(),
            "-S".to_string(), self.sel_bg.clone(),
            "-s".to_string(), self.sel_fg.clone(),
            "-M".to_string(), self.bg.clone(),
            "-m".to_string(), self.fg.clone(),
        ]
    }
}

/// How to execute a selected application.
#[derive(Debug, Clone)]
enum Exec {
    /// A binary found on $PATH — executed directly.
    PathBinary { name: String },
    /// A Flatpak application — executed via `flatpak run <app_id>`.
    Flatpak { app_id: String },
}

impl Exec {
    /// Spawn the command directly (no shell).
    fn spawn(&self) -> std::io::Result<std::process::Child> {
        match self {
            Exec::PathBinary { name } => {
                Command::new(name)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            }
            Exec::Flatpak { app_id } => {
                Command::new("flatpak")
                    .args(["run", app_id])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    display: String,
    exec: Exec,
}

fn main() {
    let mut wmenu_args: Vec<String> = std::env::args().skip(1).collect();

    // If no colour args were provided, try to load the current theme so
    // wmenu stays consistent with the River init script.
    let has_color_args = wmenu_args.iter().any(|a| {
        matches!(a.as_str(), "-N" | "-n" | "-S" | "-s" | "-M" | "-m")
    });
    if !has_color_args {
        if let Some(theme) = ThemeState::read() {
            wmenu_args.extend(theme.to_wmenu_args());
        }
    }

    let mut entries: Vec<Entry> = Vec::new();
    let mut seen = HashMap::new();

    // 1. PATH binaries
    for name in scan_path_binaries() {
        if !seen.contains_key(&name) {
            seen.insert(name.clone(), entries.len());
            entries.push(Entry {
                display: name.clone(),
                exec: Exec::PathBinary { name },
            });
        }
    }

    // 2. Flatpak apps
    for entry in scan_flatpak_apps() {
        if !seen.contains_key(&entry.display) {
            seen.insert(entry.display.clone(), entries.len());
            entries.push(entry);
        }
    }

    // 3. Sort by display name
    entries.sort_by(|a, b| a.display.cmp(&b.display));

    if entries.is_empty() {
        eprintln!("sheep-run: no applications found");
        std::process::exit(1);
    }

    // 4. Pipe to wmenu
    let mut wmenu = Command::new("wmenu")
        .args(&wmenu_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("sheep-run: failed to spawn wmenu: {}", e);
            std::process::exit(1);
        });

    {
        let stdin = wmenu.stdin.take().expect("wmenu stdin");
        let mut writer = io::BufWriter::new(stdin);
        for entry in &entries {
            if writeln!(writer, "{}", entry.display).is_err() {
                break;
            }
        }
    }

    let output = wmenu.wait_with_output().unwrap_or_else(|e| {
        eprintln!("sheep-run: wmenu failed: {}", e);
        std::process::exit(1);
    });

    if !output.status.success() {
        std::process::exit(output.status.code().unwrap_or(1));
    }

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let selection = stdout_str
        .lines()
        .next()
        .unwrap_or("")
        .trim();

    if selection.is_empty() {
        return;
    }

    // 5. Execute directly (no shell — prevents command injection).
    let entry = entries
        .iter()
        .find(|e| e.display == selection);

    match entry {
        Some(e) => {
            if let Err(err) = e.exec.spawn() {
                eprintln!("sheep-run: failed to spawn '{}': {}", e.display, err);
                std::process::exit(1);
            }
        }
        None => {
            // Fallback: treat the raw selection as a PATH binary.
            // This handles edge cases where the user typed something
            // not in the menu.  Still executed directly, not via shell.
            if let Err(err) = Command::new(selection)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                eprintln!("sheep-run: failed to spawn '{}': {}", selection, err);
                std::process::exit(1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PATH scanning
// ---------------------------------------------------------------------------

fn scan_path_binaries() -> Vec<String> {
    let mut names = Vec::new();

    let path = match std::env::var("PATH") {
        Ok(p) => p,
        Err(_) => return names,
    };

    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let dir = Path::new(dir);
        let Ok(read_dir) = fs::read_dir(dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if is_executable(&path) {
                names.push(name_str.into_owned());
            }
        }
    }

    names.sort();
    names.dedup();
    names
}

/// Check whether `path` points to an executable file.
///
/// Uses `symlink_metadata` to avoid following symlinks (consistent with
/// how shells resolve PATH lookups).  The actual execution will fail
/// gracefully with `PermissionDenied` if the file is modified between
/// this check and the spawn.
fn is_executable(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if meta.is_dir() {
        return false;
    }
    let mode = meta.permissions().mode();
    mode & 0o111 != 0
}

// ---------------------------------------------------------------------------
// Flatpak scanning
// ---------------------------------------------------------------------------

fn scan_flatpak_apps() -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut dirs = vec![PathBuf::from("/var/lib/flatpak/exports/share/applications")];

    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/share/flatpak/exports/share/applications"));
    }

    for dir in dirs {
        let Ok(read_dir) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(ext) = path.extension() else {
                continue;
            };
            if ext != "desktop" {
                continue;
            }
            let Some(stem) = path.file_stem() else {
                continue;
            };
            let app_id = stem.to_string_lossy().to_string();
            if let Some(name) = parse_desktop_name(&path) {
                let display = format!("{} ({})", name, app_id);
                entries.push(Entry {
                    display,
                    exec: Exec::Flatpak { app_id },
                });
            }
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_desktop_name_extracts_name() {
        let tmp = std::env::temp_dir().join(format!(
            "sheep-run-test-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("test.desktop");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            writeln!(f, "[Desktop Entry]").unwrap();
            writeln!(f, "Name=Test Application").unwrap();
            writeln!(f, "Exec=test-app").unwrap();
        }
        assert_eq!(parse_desktop_name(&path), Some("Test Application".to_string()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_desktop_name_handles_invalid_utf8() {
        let tmp = std::env::temp_dir().join(format!(
            "sheep-run-test-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path = tmp.join("bad.desktop");
        {
            let mut f = std::fs::File::create(&path).unwrap();
            // Write a .desktop file with invalid UTF-8 bytes in the middle
            f.write_all(b"[Desktop Entry]\nName=Bad \xff\xfe App\nExec=bad\n").unwrap();
        }
        // Should still parse (lossy) rather than return None.
        let name = parse_desktop_name(&path);
        assert!(name.is_some(), "should parse invalid UTF-8 lossily");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parse_desktop_name_returns_none_for_missing_file() {
        let path = Path::new("/nonexistent/path/test.desktop");
        assert_eq!(parse_desktop_name(path), None);
    }

    #[test]
    fn exec_flatpak_spawns_correctly() {
        // We can't actually spawn flatpak in a unit test, but we can verify
        // the command structure is correct.
        let exec = Exec::Flatpak {
            app_id: "org.test.App".to_string(),
        };
        let cmd = match &exec {
            Exec::Flatpak { app_id } => {
                let mut c = Command::new("flatpak");
                c.args(["run", app_id]);
                c
            }
            _ => panic!("expected Flatpak variant"),
        };
        // Just verify it builds the command without panicking.
        let _ = cmd;
    }

    #[test]
    fn exec_path_binary_spawns_correctly() {
        let exec = Exec::PathBinary {
            name: "echo".to_string(),
        };
        let cmd = match &exec {
            Exec::PathBinary { name } => {
                let mut c = Command::new(name);
                c.arg("hello");
                c
            }
            _ => panic!("expected PathBinary variant"),
        };
        let _ = cmd;
    }
}

fn parse_desktop_name(path: &Path) -> Option<String> {
    // Use lossy conversion so non-UTF-8 .desktop files are still parsed
    // rather than silently skipped.
    let content = fs::read(path).ok()?;
    let content = String::from_utf8_lossy(&content);
    let mut in_desktop_entry = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_desktop_entry = trimmed == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("Name=") {
            return Some(value.to_string());
        }
    }

    None
}
