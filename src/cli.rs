//! CLI argument parsing with clap.

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "sheep-rhel",
    about = "Fault-tolerant provisioner for River Classic on RHEL 10.2",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Apply a theme by name without running full install (omit value with --interactive)
    #[arg(long, value_name = "NAME", num_args = 0..=1)]
    pub theme: Option<String>,

    /// Launch interactive theme selector
    #[arg(long, default_value_t = false)]
    pub interactive: bool,

    /// Simulate all operations without mutating the system
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,

    /// Rollback all artifacts created by a previous run
    #[arg(long, default_value_t = false)]
    pub rollback: bool,

    /// Auto-install missing dependencies via dnf
    #[arg(long, default_value_t = false)]
    pub auto_install: bool,

    /// Increase log verbosity (use multiple times for more detail)
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the full provisioning lifecycle (default)
    Install,
}
