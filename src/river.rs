//! River Classic (v0.3.x) compositor lifecycle: clone, download zig 0.14, build, install, init script.

use anyhow::{bail, Context, Result};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tracing::{info, warn};

use crate::config::{build_dir, real_user_config_dir, RIVER_DEFAULT_TAG, RIVER_GIT_URL, WMENU_GIT_URL, WMENU_DEFAULT_TAG, BIN_PREFIX};
use crate::theme::generate_river_init_script;
use crate::utils::{atomic_write, chown_to_real_user, retry_with_backoff, run_command_logged, Manifest};

const RIVER_BUILD_DIR: &str = "river";
const WMENU_BUILD_DIR: &str = "wmenu";
const ZIG014_URL: &str = "https://ziglang.org/download/0.14.1/zig-x86_64-linux-0.14.1.tar.xz";
const ZIG014_DIR: &str = "zig-x86_64-linux-0.14.1";
/// SHA256 checksum of the Zig 0.14.1 tarball (x86_64-linux).
/// Verified against https://ziglang.org/download/0.14.1/
const ZIG014_SHA256: &str = "24aeeec8af16c381934a6cd7d95c807a8cb2cf7df9fa40d359aa884195c4716c";

/// Convert a `Path` to a `&str`, returning a meaningful error if the path
/// contains non-UTF-8 bytes.
fn path_to_str(path: &Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("Path is not valid UTF-8: {}", path.display()))
}

/// Verify the SHA256 checksum of a file against an expected hex digest.
async fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let output = run_command_logged("sha256sum", &[path_to_str(path)?], None).await?;
    if !output.success() {
        bail!("sha256sum command failed for {}", path.display());
    }
    // sha256sum outputs: "<hash>  <filename>"
    // We only need the first 64 hex characters.
    // Since run_command_logged only returns ExitStatus, we re-run with output capture.
    let output = tokio::process::Command::new("sha256sum")
        .arg(path_to_str(path)?)
        .output()
        .await
        .with_context(|| format!("Failed to run sha256sum on {}", path.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let actual = stdout.split_whitespace().next().unwrap_or("");
    if actual != expected {
        bail!(
            "SHA256 mismatch for {}: expected {}, got {}",
            path.display(),
            expected,
            actual
        );
    }
    info!("SHA256 verified for {}: {}", path.display(), expected);
    Ok(())
}

/// Download and extract Zig 0.14.1 into the build scratch directory.
/// River v0.3.9 was written for Zig 0.14; RHEL 10.2 ships Zig 0.16 which
/// is incompatible. Using the matching compiler avoids fragile source patches.
async fn ensure_zig014(base: &Path) -> Result<std::path::PathBuf> {
    let zig_dir = base.join(ZIG014_DIR);
    let zig_bin = zig_dir.join("zig");

    if zig_bin.exists() {
        info!("Zig 0.14.1 already present at {}", zig_bin.display());
        return Ok(zig_bin);
    }

    info!("Downloading Zig 0.14.1 for River v0.3.9 compatibility...");
    let tarball = base.join("zig-0.14.1.tar.xz");

    fs::create_dir_all(base)?;

    retry_with_backoff("download zig 0.14.1", 5, 5000, || async {
        let status = run_command_logged(
            "curl",
            &[
                "-L",
                "-o",
                path_to_str(&tarball)?,
                ZIG014_URL,
            ],
            None,
        ).await?;
        if !status.success() {
            bail!("curl download of Zig 0.14.1 failed");
        }
        Ok(())
    }).await?;

    // Verify checksum before extraction (supply-chain security).
    verify_sha256(&tarball, ZIG014_SHA256).await?;

    retry_with_backoff("extract zig 0.14.1", 5, 5000, || async {
        let status = run_command_logged(
            "tar",
            &["-xf", path_to_str(&tarball)?, "-C", path_to_str(base)?],
            None,
        ).await?;
        if !status.success() {
            bail!("tar extraction of Zig 0.14.1 failed");
        }
        Ok(())
    }).await?;

    if !zig_bin.exists() {
        bail!("Zig 0.14.1 binary not found at {} after extraction", zig_bin.display());
    }

    // Make executable just in case
    let mut perms = fs::metadata(&zig_bin)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&zig_bin, perms)?;

    info!("Zig 0.14.1 ready at {}", zig_bin.display());
    Ok(zig_bin)
}

/// Full River Classic lifecycle.
pub async fn lifecycle(manifest: &mut Manifest, dry_run: bool) -> Result<()> {
    let base = build_dir();
    let river_src = base.join(RIVER_BUILD_DIR);

    // Step 1: Clone (or switch tag if already present but wrong version)
    if !river_src.join(".git").exists() {
        info!("Cloning River from {}", RIVER_GIT_URL);
        if !dry_run {
            fs::create_dir_all(&base)?;
            retry_with_backoff("git clone River", 5, 5000, || async {
                let status = run_command_logged(
                    "git",
                    &["clone", "--branch", RIVER_DEFAULT_TAG, RIVER_GIT_URL, path_to_str(&river_src)?],
                    None,
                ).await?;
                if !status.success() {
                    bail!("git clone River exited with non-zero status");
                }
                Ok(())
            }).await?;
        }
    } else {
        info!("River source already present at {}", river_src.display());
        if !dry_run {
            let describe_output = std::process::Command::new("git")
                .args(["-C", path_to_str(&river_src)?, "describe", "--tags", "--exact-match"])
                .output();
            let current_tag = describe_output
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            if current_tag != RIVER_DEFAULT_TAG {
                warn!(
                    "River checkout is at '{}' but '{}' is required; switching...",
                    current_tag, RIVER_DEFAULT_TAG
                );
                run_command_logged(
                    "git",
                    &["-C", path_to_str(&river_src)?, "fetch", "origin", "tag", RIVER_DEFAULT_TAG, "--no-tags"],
                    None,
                ).await?;
                run_command_logged(
                    "git",
                    &["-C", path_to_str(&river_src)?, "checkout", "-f", RIVER_DEFAULT_TAG],
                    None,
                ).await?;
            }
        }
    }

    // Clear any stale Zig build cache from previous attempts
    if !dry_run {
        let zig_cache = river_src.join(".zig-cache");
        if zig_cache.exists() {
            fs::remove_dir_all(&zig_cache)?;
            info!("Cleared stale Zig build cache");
        }
    }

    // Step 2: Obtain Zig 0.14.1 (v0.3.9 requires Zig 0.14, not 0.16)
    let zig_bin = if dry_run {
        std::path::PathBuf::from("zig")
    } else {
        ensure_zig014(&base).await?
    };

    // Step 3: Build with zig
    info!("Building River Classic with zig (ReleaseSafe)");
    if !dry_run {
        retry_with_backoff("zig build River", 5, 10000, || async {
            let status = run_command_logged(
                path_to_str(&zig_bin)?,
                &["build", "-Doptimize=ReleaseSafe", "--prefix", BIN_PREFIX, "install"],
                Some(&river_src),
            ).await?;
            if !status.success() {
                bail!("zig build River exited with non-zero status");
            }
            Ok(())
        }).await?;
    }

    // Step 4: Verify binary in prefix
    let river_bin_dst = Path::new(BIN_PREFIX).join("bin").join("river");

    if !dry_run && !river_bin_dst.exists() {
        warn!("River binary not found at {} after zig build install", river_bin_dst.display());
    }

    if river_bin_dst.exists() || dry_run {
        info!("River binary installed at {}", river_bin_dst.display());
        manifest.push(crate::utils::Artifact::Binary {
            path: river_bin_dst.clone(),
        });
    }

    // Step 5: Ensure rivertile is installed (it should be built alongside river)
    let rivertile_bin_dst = Path::new(BIN_PREFIX).join("bin").join("rivertile");
    if !dry_run && !rivertile_bin_dst.exists() {
        warn!("rivertile not found at {} — layout generator may be unavailable", rivertile_bin_dst.display());
    }

    // Step 6: Baseline config directory and init script
    let config_dir = real_user_config_dir().join("river");
    let init_script = config_dir.join("init");

    if !init_script.exists() || dry_run {
        let init_contents = generate_river_init_script("rhel-red")?;
        atomic_write(&init_script, init_contents)?;
        if !dry_run {
            let mut perms = fs::metadata(&init_script)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&init_script, perms)?;
            chown_to_real_user(&init_script)?;
        }
        manifest.push(crate::utils::Artifact::File {
            path: init_script.clone(),
        });
    } else {
        info!("River init script already exists at {}", init_script.display());
    }

    if !config_dir.exists() {
        if !dry_run {
            fs::create_dir_all(&config_dir)?;
            chown_to_real_user(&config_dir)?;
        }
        manifest.push(crate::utils::Artifact::Dir {
            path: config_dir.clone(),
        });
    }

    // Step 7: Build and install wmenu launcher
    ensure_wmenu(manifest, dry_run).await?;

    // Step 8: Ensure user is in input/video groups for device access
    ensure_user_groups(dry_run).await?;

    info!("River Classic lifecycle complete");
    Ok(())
}

/// Add the real user to input/video groups so River can access devices.
async fn ensure_user_groups(dry_run: bool) -> Result<()> {
    let user = std::env::var("SUDO_USER")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "root".to_string());

    for group in &["input", "video"] {
        if dry_run {
            info!("[DRY-RUN] Would add {} to {} group", user, group);
            continue;
        }

        // Check if user is already in the group (blocking I/O wrapped in spawn_blocking).
        let user_clone = user.clone();
        let group_clone = group.to_string();
        let in_group = tokio::task::spawn_blocking(move || {
            std::process::Command::new("id")
                .args(["-nG", &user_clone])
                .output()
                .ok()
                .and_then(|o| {
                    let groups = String::from_utf8_lossy(&o.stdout);
                    Some(groups.split_whitespace().any(|g| g == group_clone))
                })
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false);

        if in_group {
            info!("User {} is already in {} group", user, group);
            continue;
        }

        info!("Adding user {} to {} group", user, group);
        let status = run_command_logged("usermod", &["-aG", group, &user], None).await?;
        if !status.success() {
            warn!("Failed to add {} to {} group; River may not have input/video access", user, group);
        } else {
            info!("Added {} to {} group — log out and back in for changes to take effect", user, group);
        }
    }

    Ok(())
}

/// Clone, build, and install wmenu (Wayland-native dmenu replacement).
async fn ensure_wmenu(manifest: &mut Manifest, dry_run: bool) -> Result<()> {
    let base = build_dir();
    let wmenu_src = base.join(WMENU_BUILD_DIR);

    if !wmenu_src.join(".git").exists() {
        info!("Cloning wmenu from {}", WMENU_GIT_URL);
        if !dry_run {
            fs::create_dir_all(&base)?;
            retry_with_backoff("git clone wmenu", 5, 5000, || async {
                let status = run_command_logged(
                    "git",
                    &["clone", "--depth", "1", "--branch", WMENU_DEFAULT_TAG, WMENU_GIT_URL, path_to_str(&wmenu_src)?],
                    None,
                ).await?;
                if !status.success() {
                    bail!("git clone wmenu failed");
                }
                Ok(())
            }).await?;
        }
    } else {
        info!("wmenu source already present at {}", wmenu_src.display());
    }

    let wmenu_bin_dst = Path::new(BIN_PREFIX).join("bin").join("wmenu");
    let wmenu_run_bin_dst = Path::new(BIN_PREFIX).join("bin").join("wmenu-run");

    if wmenu_bin_dst.exists() && wmenu_run_bin_dst.exists() {
        info!("wmenu already installed");
        return Ok(());
    }

    info!("Building wmenu with meson");
    if !dry_run {
        let build_path = wmenu_src.join("build");
        if !build_path.exists() {
            retry_with_backoff("meson setup wmenu", 5, 10000, || async {
                let status = run_command_logged(
                    "meson",
                    &["setup", "build"],
                    Some(&wmenu_src),
                ).await?;
                if !status.success() {
                    bail!("meson setup wmenu failed");
                }
                Ok(())
            }).await?;
        }

        retry_with_backoff("meson compile wmenu", 5, 10000, || async {
            let status = run_command_logged(
                "meson",
                &["compile", "-C", "build"],
                Some(&wmenu_src),
            ).await?;
            if !status.success() {
                bail!("meson compile wmenu failed");
            }
            Ok(())
        }).await?;

        retry_with_backoff("meson install wmenu", 5, 10000, || async {
            let status = run_command_logged(
                "meson",
                &["install", "-C", "build"],
                Some(&wmenu_src),
            ).await?;
            if !status.success() {
                bail!("meson install wmenu failed");
            }
            Ok(())
        }).await?;
    }

    if wmenu_bin_dst.exists() || dry_run {
        info!("wmenu installed at {}", wmenu_bin_dst.display());
        manifest.push(crate::utils::Artifact::Binary {
            path: wmenu_bin_dst.clone(),
        });
    }
    if wmenu_run_bin_dst.exists() || dry_run {
        info!("wmenu-run installed at {}", wmenu_run_bin_dst.display());
        manifest.push(crate::utils::Artifact::Binary {
            path: wmenu_run_bin_dst.clone(),
        });
    }

    Ok(())
}
