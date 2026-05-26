//! Dependency audit and DNF installation engine.

use anyhow::{bail, Result};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::DNF_PACKAGES;
use crate::utils::{is_dry_run, run_command_logged, Manifest};

/// Check if a DNF package is installed using `rpm -q`.
async fn is_package_installed(name: &str) -> bool {
    let output = match Command::new("rpm")
        .args(["-q", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        Ok(status) => status.success(),
        Err(_) => false,
    };
    output
}

/// Run dnf install with optional sudo and strict=0 fallback.
async fn dnf_install(packages: &[&str]) -> Result<()> {
    if packages.is_empty() {
        return Ok(());
    }

    if is_dry_run() {
        info!("[DRY-RUN] Would install DNF packages: {}", packages.join(", "));
        return Ok(());
    }

    let mut args = vec!["install", "-y"];
    args.extend(packages);

    info!("Running dnf {}", args.join(" "));

    // Try without sudo first (might already be root).
    let status = run_command_logged("dnf", &args, None).await?;

    if !status.success() {
        // Retry with --setopt=strict=0
        warn!("DNF failed; retrying with --setopt=strict=0");
        let mut args_strict = vec!["install", "-y", "--setopt=strict=0"];
        args_strict.extend(packages);

        let status2 = run_command_logged("dnf", &args_strict, None).await?;

        if !status2.success() {
            // Try with sudo
            warn!("DNF retry failed; attempting with sudo");
            let mut sudo_args = vec!["dnf"];
            sudo_args.extend(args_strict.iter().copied());
            let status3 = run_command_logged("sudo", &sudo_args, None).await?;

            if !status3.success() {
                bail!("DNF installation failed for packages: {}", packages.join(", "));
            }
        }
    }

    info!("DNF packages installed successfully");
    Ok(())
}

/// Audit all required dependencies and auto-install missing ones.
pub async fn audit_and_install(manifest: &mut Manifest, auto_install: bool) -> Result<()> {
    let mut missing: Vec<&str> = Vec::new();

    for pkg in DNF_PACKAGES {
        if is_package_installed(pkg).await {
            info!("Dependency OK: {}", pkg);
        } else {
            warn!("Dependency missing: {}", pkg);
            missing.push(pkg);
        }
    }

    if missing.is_empty() {
        info!("All dependencies satisfied");
        return Ok(());
    }

    if auto_install {
        info!("Auto-installing missing packages: {}", missing.join(", "));
        dnf_install(&missing).await?;

        // Re-audit: --setopt=strict=0 can silently skip missing packages.
        let mut still_missing: Vec<&str> = Vec::new();
        for pkg in &missing {
            if is_package_installed(pkg).await {
                manifest.push(crate::utils::Artifact::DnfPackage {
                    name: pkg.to_string(),
                });
            } else {
                still_missing.push(pkg);
            }
        }

        if !still_missing.is_empty() {
            bail!(
                "DNF could not install the following packages (unavailable in enabled repos): {}. \
                 You must install them manually before re-running. \
                 For RHEL 10.2, consider enabling EPEL/COPR or building from source.",
                still_missing.join(", ")
            );
        }
    } else {
        bail!(
            "Missing dependencies ({}): {}. Re-run with --auto-install or install manually.",
            missing.len(),
            missing.join(", ")
        );
    }

    info!("All dependencies satisfied after installation");
    Ok(())
}
