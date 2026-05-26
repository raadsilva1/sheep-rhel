//! sheep-rhel — Fault-tolerant provisioner for River Classic on RHEL 10.2.

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{info, warn};

mod cli;
mod config;
mod deps;
mod flatpak_fixes;
mod platform;
mod river;
mod session;
mod terminal;
mod theme;
mod utils;
mod xdg_desktop;

use cli::{Cli, Commands};
use utils::{set_dry_run, Manifest};

#[tokio::main]
async fn main() -> Result<()> {
    let args = Cli::parse();

    // Initialize tracing subscriber.
    let log_level = match args.verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(log_level)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    set_dry_run(args.dry_run);

    if args.dry_run {
        info!("=== DRY RUN MODE === No system changes will be made.");
    }

    // Rollback takes highest precedence.
    if args.rollback {
        let manifest = Manifest::load().context("Failed to load rollback manifest")?;
        if manifest.artifacts.is_empty() {
            warn!("No artifacts recorded in manifest; nothing to rollback.");
            return Ok(());
        }
        info!("Rolling back {} artifacts...", manifest.artifacts.len());
        manifest.rollback().context("Rollback failed")?;
        return Ok(());
    }

    // Theme-only mode.
    if args.theme.is_some() || args.interactive {
        let name = if args.interactive {
            theme::interactive_select().context("Interactive theme selection failed")?
        } else {
            args.theme.unwrap_or_default()
        };
        if name.is_empty() {
            eprintln!("Error: --theme requires a theme name (or use --interactive).");
            std::process::exit(2);
        }
        theme::apply_theme(&name, None).context("Failed to apply theme")?;
        return Ok(());
    }

    // Full install (default).
    match args.command {
        Some(Commands::Install) | None => {
            run_install(args.dry_run, args.auto_install).await?;
        }
    }

    Ok(())
}

async fn install_sheep_run(manifest: &mut Manifest) -> Result<()> {
    use std::path::Path;
    use anyhow::bail;

    // Resolve sheep-run relative to the current executable's directory so
    // the installer works regardless of the working directory.
    let src = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("sheep-run")))
        .filter(|p| p.exists())
        .unwrap_or_else(|| Path::new("target/release/sheep-run").to_path_buf());

    if !src.exists() {
        bail!("sheep-run binary not found at {} — run 'cargo build --release' first", src.display());
    }

    let dst = Path::new("/usr/local/bin/sheep-run");
    if dst.exists() {
        info!("sheep-run already installed at {}", dst.display());
        return Ok(());
    }

    // Wrap blocking filesystem operations in spawn_blocking.
    let src_clone = src.clone();
    let dst_clone = dst.to_path_buf();
    tokio::task::spawn_blocking(move || {
        std::fs::copy(&src_clone, &dst_clone)
            .with_context(|| format!("Failed to copy sheep-run to {}", dst_clone.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&dst_clone)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&dst_clone, perms)?;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .with_context(|| "spawn_blocking for sheep-run install panicked")??;

    manifest.push(crate::utils::Artifact::Binary {
        path: dst.to_path_buf(),
    });
    info!("Installed sheep-run at {}", dst.display());
    Ok(())
}

async fn run_install(dry_run: bool, auto_install: bool) -> Result<()> {
    info!("Starting sheep-rhel provisioning...");

    let mut manifest = Manifest::load().unwrap_or_default();

    // Step 1/6: Platform validation
    info!("Step 1/6: Validating platform...");
    platform::validate_platform()
        .context("Step 1/6: Platform validation failed")?;

    // Step 2/6: Dependency audit
    info!("Step 2/6: Auditing dependencies...");
    deps::audit_and_install(&mut manifest, auto_install)
        .await
        .context("Step 2/6: Dependency audit failed")?;

    // Step 3/6: Terminal validation
    info!("Step 3/6: Validating default terminal...");
    terminal::validate_terminal()
        .await
        .context("Step 3/6: Terminal validation failed")?;

    // Step 4/6: River lifecycle
    info!("Step 4/6: Provisioning River compositor...");
    river::lifecycle(&mut manifest, dry_run)
        .await
        .context("Step 4/6: River provisioning failed")?;

    // Step 5/6: GDM session integration
    info!("Step 5/6: Installing GDM session entry...");
    session::install_session(&mut manifest)
        .await
        .context("Step 5/6: GDM session installation failed")?;

    // Step 5b: Install sheep-run launcher
    info!("Step 5b: Installing sheep-run launcher...");
    install_sheep_run(&mut manifest)
        .await
        .context("Step 5b: sheep-run installation failed")?;

    // Step 5c: Flatpak fixes (Firefox font rendering)
    info!("Step 5c: Applying Flatpak font fixes...");
    flatpak_fixes::apply()
        .context("Step 5c: Flatpak fixes failed")?;

    // Step 5d: XDG Desktop Portal integration for GNOME apps
    info!("Step 5d: Configuring XDG Desktop Portal integration...");
    xdg_desktop::apply(&mut manifest)
        .await
        .context("Step 5d: XDG Desktop Portal configuration failed")?;

    // Step 6/6: Apply default theme
    info!("Step 6/6: Applying default Red Hat theme...");
    let theme_path = theme::apply_theme("rhel-red", Some(&mut manifest))
        .context("Step 6/6: Theme application failed")?;
    manifest.push(crate::utils::Artifact::File {
        path: theme_path,
    });

    // Save manifest for rollback
    if !dry_run {
        manifest.save().context("Failed to save rollback manifest")?;
    }

    info!("Provisioning complete! Reboot and select 'River' from GDM.");
    Ok(())
}
