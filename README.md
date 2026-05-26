# sheep-rhel

Fault-tolerant, self-contained CLI provisioner for the **River Classic** Wayland compositor on **RHEL 10.2**.

## Overview

`sheep-rhel` automates the end-to-end installation of a modern, tiling Wayland desktop stack:

- **River Classic** — A dynamic tiling Wayland compositor (v0.3.x, built with Zig)
- **Ptyxis** — The default terminal emulator on RHEL 10.2, hard-coded into all keybindings
- **GDM** — Custom Wayland session entry so you can select "River" at login
- **Red Hat Themes** — 10 built-in color themes inspired by the official Red Hat palette

## Features

- **Platform Guard** — Strictly validates RHEL 10.2 via `/etc/os-release`
- **Dependency Engine** — Audits build/run-time dependencies and optionally auto-installs via `dnf`
- **Fault Tolerance** — Exponential-backoff retry logic, atomic file operations, idempotent re-runs
- **Dry-Run Mode** — Simulate every step without mutating the system
- **Rollback** — Tracks all artifacts in a JSON manifest and can uninstall them
- **Theme Engine** — CLI flag or interactive TUI to preview and apply Red Hat-themed River configs

## Quick Start

### Build

```bash
cargo build --release
```

The resulting binary is at `target/release/sheep-rhel`.

### Run

```bash
# Full provisioning (dry-run first recommended)
sudo ./target/release/sheep-rhel --dry-run --auto-install

# Actual install
sudo ./target/release/sheep-rhel --auto-install

# Apply a theme
./target/release/sheep-rhel --theme rhel-red

# Interactive theme picker
./target/release/sheep-rhel --interactive

# Rollback everything
sudo ./target/release/sheep-rhel --rollback
```

## CLI Reference

| Flag / Command | Description |
|----------------|-------------|
| `install` | Run the full provisioning lifecycle (default) |
| `--theme <NAME>` | Apply a named theme without running full install |
| `--interactive` | Launch interactive theme selector |
| `--dry-run` | Simulate all operations without mutation |
| `--rollback` | Remove all artifacts created by a previous run |
| `--auto-install` | Automatically install missing DNF dependencies |
| `-v, --verbose` | Increase log verbosity (repeat for more detail) |

## Architecture

```
src/
├── main.rs           # CLI entrypoint and orchestration
├── cli.rs            # clap argument definitions
├── config.rs         # Constants, URLs, package lists
├── platform.rs       # RHEL 10.2 validation
├── deps.rs           # DNF dependency audit & installation
├── flatpak_fixes.rs  # Flatpak app workarounds (Firefox fonts)
├── river.rs          # River clone, zig build, init script
├── session.rs        # GDM Wayland session desktop entry
├── terminal.rs       # Ptyxis validation
├── theme.rs          # Red Hat palette theme generator + TUI picker
├── utils.rs          # Retry logic, atomic writes, rollback manifest
├── xdg_desktop.rs    # XDG Portal & GNOME app integration
└── bin/sheep-run.rs  # Unified app launcher (PATH + Flatpak)
```

## Known Issues & Workarounds

### Firefox Flatpak: no text rendered on River

RHEL 10.2 ships Firefox as a Flatpak (`org.mozilla.firefox`). On wlroots-based
compositors such as River, Firefox may open successfully but fail to render any
text (blank UI and web pages). This is caused by bitmap fonts in the
fontconfig fallback chain inside the Flatpak sandbox.

**Fix applied automatically** by `sheep-rhel` (Step 5c):

- Rejects non-scalable (bitmap) fonts via
  `~/.var/app/org.mozilla.firefox/config/fontconfig/fonts.conf`
- Copies `70-no-bitmaps.conf` into the per-app fontconfig directory
- Applies `flatpak override --filesystem=xdg-data/fonts:ro`

**After provisioning, restart Firefox** for the change to take effect.

If text still does not render after restart, force XWayland as a fallback:

```bash
flatpak override --user --env=MOZ_ENABLE_WAYLAND=0 org.mozilla.firefox
```

### GNOME App File Choosers on River

GNOME apps (Chrome, Waterfox, Nautilus, Settings) use XDG Desktop Portals for
file dialogs. On River, the portal backend services fail to start via systemd
because they require `graphical-session.target`, which is inactive in River
sessions.

**Fix applied automatically** by `sheep-rhel` (Step 5d):

- Writes `~/.config/xdg-desktop-portal/portals.conf` with explicit backend
  selection (`FileChooser` → GTK, `Settings` → GNOME)
- Installs D-Bus service overrides in `~/.local/share/dbus-1/services/` that
  disable systemd activation, forcing direct binary execution on re-activation
- Sets `GTK_USE_PORTAL=1` via `~/.config/environment.d/90-sheep-rhel-portals.conf`
- Injects direct portal startup into `~/.config/river/init` with `nohup` so
  backends survive after the init shell exits

**Limitation:** Screen sharing / screen casting requires a Mutter compositor
and is not available on River. All other portals (file chooser, print,
notification, settings, secrets) work correctly.

## Themes

All themes use the same colours for **River borders**, **wmenu**, and **sheep-run**.
When you select a theme, `sheep-rhel` persists it to
`~/.config/sheep-rhel/theme.json`; `sheep-run` reads this file on launch so
wmenu always matches the River session even if launched outside the init
script.

| Theme | Border | Background | Mood |
|-------|--------|------------|------|
| `rhel-red` | #EE0000 | #1C1C1C | Classic Red Hat |
| `charcoal` | #333333 | #1C1C1C | Dark neutral |
| `fedora-blue` | #294172 | #152030 | Fedora-inspired |
| `light-gray` | #999999 | #F0F0F0 | Light workstation |
| `accent-orange` | #E87200 | #1C1C1C | Warm accent |
| `dark-red` | #8B0000 | #0A0A0A | Deep crimson |
| `steel` | #4A5A6A | #222A30 | Cool industrial |
| `crimson-night` | #B0001A | #10080C | Midnight red *(new)* |
| `rh-silver` | #A0A8B0 | #2A2E32 | Enterprise silver *(new)* |
| `ember-glow` | #FF451A | #18100C | Warm ember *(new)* |

## Requirements

- RHEL 10.2 (Workstation or Server with GUI)
- Internet access for git cloning and cargo fetching
- `sudo` privileges (for dnf and `/usr/local/bin` / `/usr/share` writes)

## License

MIT
