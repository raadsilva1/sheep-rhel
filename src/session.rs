//! GDM Wayland session integration.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use tracing::info;

use crate::config::{GDM_SESSION_DIR, GDM_SESSION_DIR_FALLBACK};
use crate::utils::{run_command_logged, Manifest};

const DESKTOP_ENTRY_NAME: &str = "river.desktop";

/// Create and install the GDM desktop entry.
pub async fn install_session(manifest: &mut Manifest) -> Result<()> {
    let primary = Path::new(GDM_SESSION_DIR);
    let fallback = Path::new(GDM_SESSION_DIR_FALLBACK);

    let target_dir = if primary.exists() && is_writable(primary).await.unwrap_or(false) {
        primary
    } else {
        info!("Primary GDM session dir not writable, using fallback");
        if !fallback.exists() {
            fs::create_dir_all(fallback)
                .with_context(|| format!("Failed to create fallback session dir {}", fallback.display()))?;
        }
        fallback
    };

    let target = target_dir.join(DESKTOP_ENTRY_NAME);

    // Write a clean minimal desktop entry.
    //
    // Earlier attempts copied gnome-wayland.desktop and patched only the
    // default Name/Exec/TryExec/Comment fields, leaving dozens of stale
    // translated Name[xx]=GNOME on Wayland lines in the file. That
    // malformed file confused GDM 47's session selector: the gear menu
    // showed "River" but the click did not stick and the user was always
    // logged into GNOME.
    //
    // A minimal file with the exact fields GDM cares about works reliably.
    let content = generate_minimal_desktop_entry();
    crate::utils::atomic_write(&target, content)?;

    // Fix SELinux context so GDM can read the file without policy denials.
    let _ = run_command_logged("restorecon", &["-v", crate::utils::path_to_str(&target)?], None).await;

    // Clean up stale test files from earlier debugging.
    let stale_test = target_dir.join("river-gnome.desktop");
    if stale_test.exists() {
        fs::remove_file(&stale_test)?;
        info!("Removed stale test session entry {}", stale_test.display());
    }

    // If the user previously logged into a test session (e.g. river-gnome)
    // GDM/AccountsService remembers it as the saved session. When that
    // .desktop file is removed, GDM falls back to GNOME and ignores
    // manual selections. We must clear the saved session.
    clear_saved_gdm_session().await?;

    // Also remove any legacy .dmrc that might hold a stale session name.
    if let Some(home) = std::env::var("SUDO_USER")
        .ok()
        .and_then(|u| {
            std::process::Command::new("getent")
                .args(["passwd", &u])
                .output()
                .ok()
                .and_then(|o| {
                    String::from_utf8(o.stdout)
                        .ok()
                        .and_then(|s| s.split(':').nth(5).map(|h| h.to_string()))
                })
        })
        .or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().to_string()))
    {
        let dmrc = Path::new(&home).join(".dmrc");
        if dmrc.exists() {
            fs::remove_file(&dmrc)?;
            info!("Removed stale .dmrc {}", dmrc.display());
        }
    }

    manifest.push(crate::utils::Artifact::File {
        path: target.clone(),
    });

    info!("Installed GDM session entry at {}", target.display());
    Ok(())
}

/// Clear any saved session preference in AccountsService so GDM does not
/// try to restore a stale session (e.g. "river-gnome") that no longer
/// exists and silently fall back to GNOME.
async fn clear_saved_gdm_session() -> Result<()> {
    let uid = std::env::var("SUDO_UID")
        .ok()
        .or_else(|| std::env::var("PKEXEC_UID").ok())
        .or_else(|| {
            std::env::var("SUDO_USER").ok().and_then(|u| {
                std::process::Command::new("id")
                    .args(["-u", &u])
                    .output()
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string()))
            })
        })
        .unwrap_or_else(|| "1000".to_string());

    let object_path = format!("/org/freedesktop/Accounts/User{}", uid);

    for prop in ["Session", "XSession"] {
        let status = run_command_logged(
            "busctl",
            &[
                "--system",
                "call",
                "org.freedesktop.Accounts",
                &object_path,
                "org.freedesktop.Accounts.User",
                &format!("Set{}", prop),
                "s",
                "",
            ],
            None,
        )
        .await?;

        if status.success() {
            info!("Cleared AccountsService {} for uid {}", prop, uid);
        } else {
            info!(
                "Could not clear AccountsService {} for uid {} (may already be empty)",
                prop, uid
            );
        }
    }

    Ok(())
}

/// Check whether the current process can write to a directory.
/// Uses libc `access` instead of metadata permissions, which gives the
/// correct answer even when the directory is owned by root.
async fn is_writable(path: &Path) -> Result<bool> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let bytes = path.as_os_str().as_encoded_bytes();
        let c_path = std::ffi::CString::new(bytes)
            .with_context(|| format!("Path contains null byte: {}", path.display()))?;
        // SAFETY: `access` is a POSIX call with no thread-safety issues.
        let ok = unsafe { libc::access(c_path.as_ptr(), libc::W_OK) == 0 };
        Ok(ok)
    })
    .await
    .unwrap_or_else(|e| {
        tracing::warn!("is_writable task panicked: {}", e);
        Ok(false)
    })
}

fn generate_minimal_desktop_entry() -> String {
    // GDM 47 on RHEL 10.2 requires specific fields to properly register
    // and display a Wayland session. The Name/Comment fields below match
    // the exact working format discovered through iterative testing.
    // Using "River" as the Name alone caused session selection to fail.
    r#"[Desktop Entry]
Name=River GNOME
Comment=This session logs you into GNOME
Exec=/usr/local/bin/river
Type=Application
DesktopNames=GNOME;river
X-GDM-SessionRegisters=true
TryExec=/usr/local/bin/river
X-GDM-CanRunHeadless=true
"#
    .to_string()
}
