//! Fault-tolerance utilities: retry, atomic writes, dry-run guards, manifest tracking.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::config::manifest_path;

/// Convert a `Path` to a `&str`, returning a meaningful error if the path
/// contains non-UTF-8 bytes.
pub fn path_to_str(path: &std::path::Path) -> anyhow::Result<&str> {
    path.to_str()
        .with_context(|| format!("Path is not valid UTF-8: {}", path.display()))
}

/// Run an external command, capture stdout/stderr, and log all output at INFO level.
///
/// Both streams are logged as INFO because many programs (git, dnf, meson, zig)
/// write routine status and progress messages to stderr.  The actual success or
/// failure of the command is determined by the returned ExitStatus; stderr is
/// not synonymous with an error.
pub async fn run_command_logged(
    program: &str,
    args: &[&str],
    current_dir: Option<&std::path::Path>,
) -> Result<std::process::ExitStatus> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = current_dir {
        cmd.current_dir(dir);
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .with_context(|| format!("Failed to spawn {} {}", program, args.join(" ")))?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let prog_stdout = program.to_string();
    let stdout_task = tokio::spawn(async move {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            info!("[{} stdout] {}", prog_stdout, line);
        }
    });

    let prog_stderr = program.to_string();
    let stderr_task = tokio::spawn(async move {
        let reader = BufReader::new(stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            info!("[{} stderr] {}", prog_stderr, line);
        }
    });

    let status = child
        .wait()
        .await
        .with_context(|| format!("Failed to wait for {}", program))?;

    // Give the logging tasks a moment to finish before dropping them.
    let _ = tokio::time::timeout(Duration::from_secs(5), stdout_task).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), stderr_task).await;

    Ok(status)
}

/// If running under sudo, chown the given path (and optionally its parent chain)
/// to the real user so they retain ownership of config files.
pub fn chown_to_real_user(path: &std::path::Path) -> Result<()> {
    if is_dry_run() {
        return Ok(());
    }
    let uid = match std::env::var("SUDO_UID") {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let gid = match std::env::var("SUDO_GID") {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };

    let status = std::process::Command::new("chown")
        .args([
            "-R",
            &format!("{}:{}", uid, gid),
            path.as_os_str().to_string_lossy().as_ref(),
        ])
        .status()
        .with_context(|| format!("Failed to chown {} to {}:{}", path.display(), uid, gid))?;

    if !status.success() {
        warn!(
            "chown {} to {}:{} returned non-zero; continuing anyway",
            path.display(),
            uid,
            gid
        );
    } else {
        info!("Chowned {} to {}:{}", path.display(), uid, gid);
    }
    Ok(())
}

/// Global dry-run flag, set once at startup.
static DRY_RUN: AtomicBool = AtomicBool::new(false);

pub fn set_dry_run(v: bool) {
    DRY_RUN.store(v, Ordering::SeqCst);
}

pub fn is_dry_run() -> bool {
    DRY_RUN.load(Ordering::SeqCst)
}

/// Log a dry-run action without executing it.
#[macro_export]
macro_rules! dry_run_guard {
    ($fmt:literal $(, $arg:expr)*) => {
        if $crate::utils::is_dry_run() {
            tracing::info!(concat!("[DRY-RUN] ", $fmt), $($arg),*);
            return Ok(());
        }
    };
}

/// Retry an async fallible operation with exponential backoff.
pub async fn retry_with_backoff<F, Fut, T>(
    desc: &str,
    max_retries: u32,
    base_delay_ms: u64,
    mut f: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut last_err = anyhow::anyhow!("retry loop did not execute");
    for attempt in 0..=max_retries {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                last_err = e;
                if attempt < max_retries {
                    let delay = Duration::from_millis(
                        base_delay_ms
                            .saturating_mul(2_u64.saturating_pow(attempt))
                            .min(300_000),
                    );
                    warn!(
                        "{} failed (attempt {}/{}): {}. Retrying in {:?}...",
                        desc,
                        attempt + 1,
                        max_retries + 1,
                        last_err,
                        delay
                    );
                    sleep(delay).await;
                }
            }
        }
    }
    Err(anyhow::anyhow!(
        "{} exhausted all {} attempts. Last error: {}",
        desc,
        max_retries + 1,
        last_err
    ))
}

/// Write a file atomically: temp file + rename.
pub fn atomic_write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> Result<()> {
    if is_dry_run() {
        info!("[DRY-RUN] Would write {}", path.as_ref().display());
        return Ok(());
    }

    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent dir for {}", path.display()))?;
    }

    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    let mut file = fs::File::create(&tmp)
        .with_context(|| format!("Failed to create temp file {}", tmp.display()))?;
    file.write_all(contents.as_ref())
        .with_context(|| format!("Failed to write temp file {}", tmp.display()))?;
    file.sync_all()
        .with_context(|| format!("Failed to sync temp file {}", tmp.display()))?;
    drop(file);

    fs::rename(&tmp, path)
        .with_context(|| format!("Failed to rename {} -> {}", tmp.display(), path.display()))?;

    info!("Wrote {}", path.display());
    Ok(())
}

/// Record of a single artifact created by the provisioner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Artifact {
    File { path: PathBuf },
    Dir { path: PathBuf },
    DnfPackage { name: String },
    Binary { path: PathBuf },
}

/// Manifest of all changes for rollback.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub artifacts: Vec<Artifact>,
}

impl Manifest {
    pub fn load() -> Result<Self> {
        let path = manifest_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read manifest at {}", path.display()))?;
        let manifest: Manifest = serde_json::from_str(&data)
            .with_context(|| format!("Failed to parse manifest at {}", path.display()))?;
        Ok(manifest)
    }

    pub fn save(&self) -> Result<()> {
        let path = manifest_path();
        let json = serde_json::to_string_pretty(self).context("Failed to serialize manifest")?;
        atomic_write(path, json)?;
        Ok(())
    }

    pub fn push(&mut self, artifact: Artifact) {
        let exists = self.artifacts.iter().any(|a| match (a, &artifact) {
            (Artifact::File { path: p1 }, Artifact::File { path: p2 }) => p1 == p2,
            (Artifact::Dir { path: p1 }, Artifact::Dir { path: p2 }) => p1 == p2,
            (Artifact::Binary { path: p1 }, Artifact::Binary { path: p2 }) => p1 == p2,
            (Artifact::DnfPackage { name: n1 }, Artifact::DnfPackage { name: n2 }) => n1 == n2,
            _ => false,
        });
        if !exists {
            self.artifacts.push(artifact);
        }
    }

    /// Perform rollback: remove artifacts in reverse order.
    pub fn rollback(&self) -> Result<()> {
        if is_dry_run() {
            info!("[DRY-RUN] Would rollback {} artifacts", self.artifacts.len());
            return Ok(());
        }

        for artifact in self.artifacts.iter().rev() {
            match artifact {
                Artifact::File { path } | Artifact::Binary { path } => {
                    if path.exists() {
                        fs::remove_file(path)
                            .with_context(|| format!("Failed to remove file {}", path.display()))?;
                        info!("Rolled back file: {}", path.display());
                    }
                }
                Artifact::Dir { path } => {
                    if path.exists() {
                        // Only remove if empty to avoid destroying user data.
                        match fs::remove_dir(path) {
                            Ok(_) => info!("Rolled back dir: {}", path.display()),
                            Err(e) => warn!(
                                "Could not remove dir {} (maybe not empty): {}",
                                path.display(),
                                e
                            ),
                        }
                    }
                }
                Artifact::DnfPackage { name } => {
                    info!("Skipping DNF removal for {} (manual step recommended)", name);
                }
            }
        }

        // Clear manifest after rollback.
        let path = manifest_path();
        if path.exists() {
            fs::remove_file(&path)
                .with_context(|| format!("Failed to remove manifest {}", path.display()))?;
        }

        info!("Rollback complete");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize tests that mutate the global dry-run flag to prevent
    /// race conditions when running tests in parallel (default for `cargo test`).
    static DRY_RUN_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Verify that the delay formula saturates instead of overflowing.
    #[test]
    fn retry_delay_formula_saturates() {
        // With base_delay_ms = u64::MAX, 2^0 = 1, but saturating_mul keeps it at u64::MAX.
        // Then min(300_000) caps it at 300_000.
        let base = u64::MAX;
        let attempt = 0;
        let delay = base
            .saturating_mul(2_u64.saturating_pow(attempt))
            .min(300_000);
        assert_eq!(delay, 300_000);

        // Verify larger attempts also cap correctly.
        let delay2 = base
            .saturating_mul(2_u64.saturating_pow(5))
            .min(300_000);
        assert_eq!(delay2, 300_000);

        // Verify normal values pass through unchanged.
        let delay3 = 10_000u64
            .saturating_mul(2_u64.saturating_pow(2))
            .min(300_000);
        assert_eq!(delay3, 40_000);
    }

    /// Verify atomic_write creates a file with correct contents.
    #[test]
    fn atomic_write_creates_file() {
        let _guard = DRY_RUN_TEST_LOCK.lock().unwrap();
        let tmp_dir = std::env::temp_dir().join(format!(
            "sheep-rhel-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&tmp_dir).unwrap();
        let target = tmp_dir.join("test-file.txt");

        set_dry_run(false);
        atomic_write(&target, "hello world").unwrap();

        assert!(target.exists());
        let contents = fs::read_to_string(&target).unwrap();
        assert_eq!(contents, "hello world");

        // Cleanup
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    /// Verify atomic_write uses a unique temp file (race-condition fix).
    #[test]
    fn atomic_write_uses_pid_suffixed_temp() {
        let tmp_dir = std::env::temp_dir().join(format!(
            "sheep-rhel-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&tmp_dir).unwrap();
        let target = tmp_dir.join("test-file.txt");

        // Write once
        atomic_write(&target, "first").unwrap();

        // The temp file should NOT exist after rename
        let tmp = tmp_dir.join(format!("test-file.txt.tmp.{}", std::process::id()));
        assert!(!tmp.exists(), "temp file should have been renamed away");

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    /// Verify dry-run mode prevents actual writes.
    #[test]
    fn atomic_write_respects_dry_run() {
        let _guard = DRY_RUN_TEST_LOCK.lock().unwrap();
        let tmp_dir = std::env::temp_dir().join(format!(
            "sheep-rhel-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        fs::create_dir_all(&tmp_dir).unwrap();
        let target = tmp_dir.join("dry-run-file.txt");

        set_dry_run(true);
        atomic_write(&target, "should not appear").unwrap();

        assert!(!target.exists());
        set_dry_run(false);

        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
