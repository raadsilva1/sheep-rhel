//! Terminal validation: ensure Ptyxis is available.

use anyhow::{bail, Result};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{info, warn};

use crate::config::DEFAULT_TERMINAL;

/// Verify that `ptyxis` exists in PATH.
pub async fn validate_terminal() -> Result<()> {
    match Command::new("which")
        .arg(DEFAULT_TERMINAL)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        Ok(status) if status.success() => {
            info!("Default terminal found: {}", DEFAULT_TERMINAL);
            Ok(())
        }
        _ => {
            warn!(
                "Default terminal '{}' not found in PATH. \
                 River keybindings may fail to launch a terminal.",
                DEFAULT_TERMINAL
            );
            bail!("Terminal '{}' is missing from PATH", DEFAULT_TERMINAL);
        }
    }
}
