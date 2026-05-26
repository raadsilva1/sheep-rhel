//! Centralized configuration constants for the provisioner.

use std::path::PathBuf;

/// Returns the home directory of the user invoking sudo, or the current user.
pub fn real_user_home() -> PathBuf {
    if let Ok(sudo_user) = std::env::var("SUDO_USER") {
        if let Ok(output) = std::process::Command::new("getent")
            .args(["passwd", &sudo_user])
            .output()
        {
            let line = String::from_utf8_lossy(&output.stdout);
            if let Some(home) = line.trim().split(':').nth(5) {
                if !home.is_empty() {
                    return PathBuf::from(home);
                }
            }
        }
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

/// Returns the real user's XDG config dir (respects XDG_CONFIG_HOME, handles sudo).
pub fn real_user_config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg);
    }
    real_user_home().join(".config")
}

/// Returns the real user's XDG data dir (respects XDG_DATA_HOME, handles sudo).
pub fn real_user_data_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg);
    }
    real_user_home().join(".local").join("share")
}

/// Expected OS release values.
pub const EXPECTED_OS_ID: &str = "rhel";
pub const EXPECTED_VERSION_ID: &str = "10.2";

/// Git repositories and references.
pub const RIVER_GIT_URL: &str = "https://codeberg.org/river/river.git";
pub const RIVER_DEFAULT_TAG: &str = "v0.3.9";

pub const WMENU_GIT_URL: &str = "https://codeberg.org/adnano/wmenu.git";
pub const WMENU_DEFAULT_TAG: &str = "main";

pub const SWAYBG_GIT_URL: &str = "https://github.com/swaywm/swaybg.git";
pub const SWAYBG_DEFAULT_TAG: &str = "v1.2.2";

/// Installation prefix for software (binaries go in $prefix/bin).
pub const BIN_PREFIX: &str = "/usr/local";

/// GDM wayland-sessions directory.
pub const GDM_SESSION_DIR: &str = "/usr/share/wayland-sessions";
pub const GDM_SESSION_DIR_FALLBACK: &str = "/usr/local/share/wayland-sessions";

/// Manifest path for rollback tracking.
pub fn manifest_path() -> PathBuf {
    real_user_data_dir().join("sheep-rhel").join("manifest.json")
}

/// Build scratch directory.
pub fn build_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("sheep-rhel")
        .join("build")
}

/// DNF packages required for River Classic.
pub const DNF_PACKAGES: &[&str] = &[
    "wayland-devel",
    "wayland-protocols-devel",
    "wlroots",
    "wlroots-devel",
    "git",
    "gcc",
    "gcc-c++",
    "libxkbcommon-devel",
    "libinput-devel",
    "pixman-devel",
    "mesa-libEGL-devel",
    "libseat-devel",
    "pango-devel",
    "cairo-devel",
    "libevdev-devel",
    "meson",
    "ninja-build",
    "pkgconf-pkg-config",
    "scdoc",
    "curl",
    "zig",
];

/// Default terminal emulator.
pub const DEFAULT_TERMINAL: &str = "ptyxis";
