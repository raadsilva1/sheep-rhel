//! Platform validation: enforce RHEL 10.2.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use tracing::{info, warn};

use crate::config::{EXPECTED_OS_ID, EXPECTED_VERSION_ID};

/// Parse /etc/os-release and abort if not RHEL 10.2.
pub fn validate_platform() -> Result<()> {
    let path = Path::new("/etc/os-release");
    if !path.exists() {
        bail!("/etc/os-release not found — cannot determine platform");
    }

    let contents =
        fs::read_to_string(path).with_context(|| "Failed to read /etc/os-release")?;

    let mut id: Option<String> = None;
    let mut version_id: Option<String> = None;

    for line in contents.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim().trim_matches('"').to_string();
            match key {
                "ID" => id = Some(value),
                "VERSION_ID" => version_id = Some(value),
                _ => {}
            }
        }
    }

    let id = id.ok_or_else(|| anyhow::anyhow!("ID field missing in /etc/os-release"))?;
    let version_id =
        version_id.ok_or_else(|| anyhow::anyhow!("VERSION_ID field missing in /etc/os-release"))?;

    info!("Detected platform: {} {}", id, version_id);

    if id != EXPECTED_OS_ID {
        bail!(
            "Platform mismatch: expected ID='{}', found ID='{}'. \
             This tool is strictly intended for RHEL 10.2.",
            EXPECTED_OS_ID,
            id
        );
    }

    if version_id != EXPECTED_VERSION_ID {
        warn!(
            "Version mismatch: expected VERSION_ID='{}', found VERSION_ID='{}'. \
             This tool is designed for RHEL 10.2; proceeding may cause issues.",
            EXPECTED_VERSION_ID,
            version_id
        );
        // Strict enforcement: both ID and VERSION_ID must match.
        bail!(
            "Platform mismatch: expected VERSION_ID='{}', found VERSION_ID='{}'. \
             This tool is strictly intended for RHEL 10.2.",
            EXPECTED_VERSION_ID,
            version_id
        );
    }

    info!("Platform validation passed: RHEL 10.2 confirmed");
    Ok(())
}
