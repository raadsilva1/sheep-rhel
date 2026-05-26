//! Flatpak-specific workarounds for RHEL 10.
//!
//! Firefox Flatpak on wlroots compositors (River) may fail to render any text
//! when bitmap fonts are present in the fontconfig fallback chain.  This
//! module installs a per-app fontconfig snippet that rejects non-scalable
//! fonts and applies filesystem overrides so host fonts are visible inside the
//! sandbox.
//!
//! # Rollback Limitation
//!
//! Flatpak overrides (`flatpak override`) are stored in Flatpak's internal
//! state (`~/.local/share/flatpak/overrides/`) and are **not** tracked in the
//! sheep-rhel manifest.  A rollback will remove the fontconfig files but will
//! not revert the Flatpak filesystem override.  This is harmless — the override
//! only grants read-only access to fonts and does not affect functionality.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use tracing::info;

use crate::config::real_user_home;

const FIREFOX_FONTCONFIG: &str = r#"<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "fonts.dtd">
<fontconfig>
  <!-- Disable bitmap fonts to fix text rendering in Firefox Flatpak -->
  <selectfont>
    <rejectfont>
      <pattern>
        <patelt name="scalable">
          <bool>false</bool>
        </patelt>
      </pattern>
    </rejectfont>
  </selectfont>
</fontconfig>
"#;

/// Apply all Flatpak fixes (currently Firefox font rendering).
pub fn apply() -> Result<()> {
    fix_firefox_fonts()?;
    Ok(())
}

fn fix_firefox_fonts() -> Result<()> {
    // Use real_user_home() so the fix is written to the invoking user's
    // home directory even when sheep-rhel is run with sudo.
    let conf_dir = real_user_home().join(".var/app/org.mozilla.firefox/config/fontconfig");

    let conf_d = conf_dir.join("conf.d");
    let fonts_conf = conf_dir.join("fonts.conf");

    fs::create_dir_all(&conf_d)
        .with_context(|| format!("Failed to create {}", conf_d.display()))?;

    if !fonts_conf.exists() {
        fs::write(&fonts_conf, FIREFOX_FONTCONFIG)
            .with_context(|| format!("Failed to write {}", fonts_conf.display()))?;
        info!("Wrote Firefox bitmap-font reject config to {}", fonts_conf.display());
    } else {
        info!("Firefox fontconfig already exists at {}", fonts_conf.display());
    }

    // Copy system 70-no-bitmaps.conf if available.
    let system_no_bitmaps = PathBuf::from("/usr/share/fontconfig/conf.avail/70-no-bitmaps.conf");
    let target_no_bitmaps = conf_d.join("70-no-bitmaps.conf");
    if system_no_bitmaps.exists() && !target_no_bitmaps.exists() {
        fs::copy(&system_no_bitmaps, &target_no_bitmaps)
            .with_context(|| format!("Failed to copy {}", system_no_bitmaps.display()))?;
        info!("Copied 70-no-bitmaps.conf to {}", target_no_bitmaps.display());
    }

    // Apply Flatpak filesystem overrides so host fonts are readable.
    // We intentionally do NOT override /usr/share/fonts because Flatpak
    // refuses to share paths under /usr ("Path /usr is reserved by Flatpak").
    // Host fonts are already exposed via /run/host/fonts by the runtime.
    let status = std::process::Command::new("flatpak")
        .args([
            "override",
            "--user",
            "--filesystem=xdg-data/fonts:ro",
            "org.mozilla.firefox",
        ])
        .status()
        .context("Failed to run flatpak override")?;

    if !status.success() {
        tracing::warn!("flatpak override returned non-zero exit code");
    } else {
        info!("Applied Flatpak font override for Firefox");
    }

    info!(
        "Firefox font fix applied. Restart Firefox for changes to take effect. \
         If text still does not render, try: \
         flatpak override --user --env=MOZ_ENABLE_WAYLAND=0 org.mozilla.firefox"
    );

    Ok(())
}
