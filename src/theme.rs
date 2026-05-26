//! Red Hat palette theme generator and selector for River Classic.

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use std::time::Duration;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color as RatColor, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use tracing::info;

use crate::config::{real_user_config_dir, DEFAULT_TERMINAL};
use crate::utils::{atomic_write, chown_to_real_user};
use crate::xdg_desktop::gnome_services_init_snippet;

/// ARGB hex color representation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Color(pub u32);

impl Color {
    pub const fn new(a: u8, r: u8, g: u8, b: u8) -> Self {
        Color(((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    /// Returns the color as `RRGGBB` (no alpha, no prefix).
    pub fn to_rrggbb(&self) -> String {
        let r = ((self.0 >> 16) & 0xFF) as u8;
        let g = ((self.0 >> 8) & 0xFF) as u8;
        let b = (self.0 & 0xFF) as u8;
        format!("{:02x}{:02x}{:02x}", r, g, b)
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{:08x}", self.0)
    }
}

/// A complete Red Hat palette theme.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: &'static str,
    pub window_border: Color,
    pub message_bg: Color,
    /// Text color for readable contrast on message_bg.
    pub text_color: Color,
    /// Accent color (reserved for future use).
    #[allow(dead_code)]
    pub accent: Color,
}

/// Built-in Red Hat palette themes.
pub const THEMES: &[Theme] = &[
    Theme {
        name: "rhel-red",
        window_border: Color::new(0xff, 0xee, 0x00, 0x00),
        message_bg: Color::new(0xff, 0x1c, 0x1c, 0x1c),
        text_color: Color::new(0xff, 0xff, 0xff, 0xff),
        accent: Color::new(0xff, 0xcc, 0x00, 0x00),
    },
    Theme {
        name: "charcoal",
        window_border: Color::new(0xff, 0x33, 0x33, 0x33),
        message_bg: Color::new(0xff, 0x1c, 0x1c, 0x1c),
        text_color: Color::new(0xff, 0xdd, 0xdd, 0xdd),
        accent: Color::new(0xff, 0xee, 0x00, 0x00),
    },
    Theme {
        name: "fedora-blue",
        window_border: Color::new(0xff, 0x29, 0x41, 0x72),
        message_bg: Color::new(0xff, 0x15, 0x20, 0x30),
        text_color: Color::new(0xff, 0xe0, 0xe8, 0xf0),
        accent: Color::new(0xff, 0x51, 0x79, 0xa8),
    },
    Theme {
        name: "light-gray",
        window_border: Color::new(0xff, 0x99, 0x99, 0x99),
        message_bg: Color::new(0xff, 0xf0, 0xf0, 0xf0),
        text_color: Color::new(0xff, 0x1c, 0x1c, 0x1c),
        accent: Color::new(0xff, 0xee, 0x00, 0x00),
    },
    Theme {
        name: "accent-orange",
        window_border: Color::new(0xff, 0xe8, 0x72, 0x00),
        message_bg: Color::new(0xff, 0x1c, 0x1c, 0x1c),
        text_color: Color::new(0xff, 0xff, 0xff, 0xff),
        accent: Color::new(0xff, 0xff, 0x99, 0x33),
    },
    Theme {
        name: "dark-red",
        window_border: Color::new(0xff, 0x8b, 0x00, 0x00),
        message_bg: Color::new(0xff, 0x0a, 0x0a, 0x0a),
        text_color: Color::new(0xff, 0xee, 0xee, 0xee),
        accent: Color::new(0xff, 0xcc, 0x33, 0x33),
    },
    Theme {
        name: "steel",
        window_border: Color::new(0xff, 0x4a, 0x5a, 0x6a),
        message_bg: Color::new(0xff, 0x22, 0x2a, 0x30),
        text_color: Color::new(0xff, 0xcc, 0xd4, 0xdc),
        accent: Color::new(0xff, 0xee, 0x00, 0x00),
    },
    // ── New themes ──────────────────────────────────────────────────────────
    Theme {
        name: "crimson-night",
        window_border: Color::new(0xff, 0xb0, 0x00, 0x1a),
        message_bg: Color::new(0xff, 0x10, 0x08, 0x0c),
        text_color: Color::new(0xff, 0xe8, 0xd8, 0xd8),
        accent: Color::new(0xff, 0xff, 0x33, 0x55),
    },
    Theme {
        name: "rh-silver",
        window_border: Color::new(0xff, 0xa0, 0xa8, 0xb0),
        message_bg: Color::new(0xff, 0x2a, 0x2e, 0x32),
        text_color: Color::new(0xff, 0xd8, 0xdc, 0xe0),
        accent: Color::new(0xff, 0xee, 0x00, 0x00),
    },
    Theme {
        name: "ember-glow",
        window_border: Color::new(0xff, 0xff, 0x45, 0x1a),
        message_bg: Color::new(0xff, 0x18, 0x10, 0x0c),
        text_color: Color::new(0xff, 0xf5, 0xe8, 0xd8),
        accent: Color::new(0xff, 0xff, 0xaa, 0x33),
    },
];

/// Find a theme by name.
pub fn find_theme(name: &str) -> Option<&'static Theme> {
    THEMES.iter().find(|t| t.name.eq_ignore_ascii_case(name))
}

/// List all theme names.
pub fn list_theme_names() -> Vec<&'static str> {
    THEMES.iter().map(|t| t.name).collect()
}

/// Persist the currently selected theme so external tools (e.g. sheep-run)
/// can read it and stay consistent with the River init script.
pub fn write_theme_state(theme_name: &str) -> Result<PathBuf> {
    let theme = find_theme(theme_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown theme: {}", theme_name))?;

    let dir = real_user_config_dir().join("sheep-rhel");
    if !crate::utils::is_dry_run() {
        fs::create_dir_all(&dir)?;
        chown_to_real_user(&dir)?;
    }

    let path = dir.join("theme.json");
    let state = ThemeState {
        name: theme.name.to_string(),
        bg: theme.message_bg.to_rrggbb(),
        fg: theme.text_color.to_rrggbb(),
        sel_bg: theme.window_border.to_rrggbb(),
        sel_fg: theme.text_color.to_rrggbb(),
    };
    let json = serde_json::to_string_pretty(&state)?;
    atomic_write(&path, json)?;

    if !crate::utils::is_dry_run() {
        chown_to_real_user(&path)?;
    }

    Ok(path)
}

/// Read the persisted theme state, if any.
#[allow(dead_code)]
pub fn read_theme_state() -> Option<ThemeState> {
    let path = real_user_config_dir().join("sheep-rhel/theme.json");
    let data = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&data).ok()
}

/// Serializable snapshot of the current theme for external consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeState {
    pub name: String,
    pub bg: String,
    pub fg: String,
    pub sel_bg: String,
    pub sel_fg: String,
}

/// Generate a full River Classic init script for the given theme.
#[must_use]
pub fn generate_river_init_script(theme_name: &str) -> Result<String> {
    let theme = find_theme(theme_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown theme: {}", theme_name))?;

    // For unfocused borders we dim the focused border by ~50% opacity.
    let unfocused = Color((theme.window_border.0 & 0x00FFFFFF) | 0x66000000);

    let bg = theme.message_bg.to_rrggbb();
    let fg = theme.text_color.to_rrggbb();
    let sel_bg = theme.window_border.to_rrggbb();
    let sel_fg = theme.text_color.to_rrggbb();

    let launcher_cmd = format!(
        "sheep-run -N {} -n {} -S {} -s {} -M {} -m {}",
        bg, fg, sel_bg, sel_fg, bg, fg
    );

    Ok(format!(
        r#"#!/bin/bash
# Auto-generated by sheep-rhel
# River Classic (v0.3.x) init script
# Theme: {}

# ═══════════════════════════════════════════════════════
# GNOME application integration
# ═══════════════════════════════════════════════════════
{}

# ═══════════════════════════════════════════════════════
# Theme colors
# ═══════════════════════════════════════════════════════
riverctl border-color-focused {}
riverctl border-color-unfocused {}

# Wallpaper — use redhat.png if present, otherwise fallback to theme background
if [ -f "$HOME/Pictures/Wallpaper/redhat.png" ]; then
    swaybg -o '*' -i "$HOME/Pictures/Wallpaper/redhat.png" -m fill &
else
    riverctl background-color {}
fi

# ═══════════════════════════════════════════════════════
# Window appearance
# ═══════════════════════════════════════════════════════
riverctl border-width 2

# ═══════════════════════════════════════════════════════
# Keyboard repeat rate (Hz) and delay (ms)
# ═══════════════════════════════════════════════════════
riverctl keyboard-repeat 50 300

# ═══════════════════════════════════════════════════════
# Layout generator (rivertile is bundled with River Classic)
# ═══════════════════════════════════════════════════════
riverctl default-layout rivertile
rivertile -view-padding 6 -outer-padding 6 &

# ═══════════════════════════════════════════════════════
# Key bindings
# ═══════════════════════════════════════════════════════

# Terminal (spawn a new window every time)
riverctl map normal Super Return spawn "{} --new-window"

# Window management
riverctl map normal Super Q close
riverctl map normal Super J focus-view next
riverctl map normal Super K focus-view previous
riverctl map normal Super+Shift J swap next
riverctl map normal Super+Shift K swap previous

# Output (monitor) focus
riverctl map normal Super Period focus-output next
riverctl map normal Super Comma focus-output previous
riverctl map normal Super+Shift Period send-to-output next
riverctl map normal Super+Shift Comma send-to-output previous

# Tags (workspaces) — 1 through 9, 0 selects all
for i in $(seq 1 9); do
    tags=$((1 << (i - 1)))
    riverctl map normal Super         $i set-focused-tags     $tags
    riverctl map normal Super+Shift   $i set-view-tags        $tags
    riverctl map normal Super+Control $i toggle-focused-tags  $tags
    riverctl map normal Super+Shift+Control $i toggle-view-tags $tags
done
riverctl map normal Super 0 set-focused-tags $(( (1 << 32) - 1 ))

# Layout control
riverctl map normal Super H   send-layout-cmd rivertile "main-ratio -0.05"
riverctl map normal Super L   send-layout-cmd rivertile "main-ratio +0.05"
riverctl map normal Super+Shift H send-layout-cmd rivertile "main-count +1"
riverctl map normal Super+Shift L send-layout-cmd rivertile "main-count -1"

# Fullscreen and floating
riverctl map normal Super         F toggle-fullscreen
riverctl map normal Super+Shift   Space toggle-float

# Themed launcher (sheep-run with Flatpak support)
riverctl map normal Super D spawn "{}"

# Exit River
riverctl map normal Super+Shift E exit
"#,
        theme.name,
        gnome_services_init_snippet(),
        theme.window_border,
        unfocused,
        theme.message_bg,
        DEFAULT_TERMINAL,
        launcher_cmd,
    ))
}

/// Write the selected theme to the River init script.
///
/// If `manifest` is provided, both the init script and the theme state file
/// are recorded for rollback.
#[must_use]
pub fn apply_theme(theme_name: &str, manifest: Option<&mut crate::utils::Manifest>) -> Result<PathBuf> {
    let contents = generate_river_init_script(theme_name)?;

    let config_dir = real_user_config_dir().join("river");
    if !config_dir.exists() {
        if !crate::utils::is_dry_run() {
            fs::create_dir_all(&config_dir)?;
            chown_to_real_user(&config_dir)?;
        }
    }

    let init_path = config_dir.join("init");
    atomic_write(&init_path, contents)?;

    if !crate::utils::is_dry_run() {
        let mut perms = fs::metadata(&init_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&init_path, perms)?;
        chown_to_real_user(&init_path)?;
    }

    // Persist theme state so sheep-run and other tools stay in sync.
    match write_theme_state(theme_name) {
        Ok(path) => {
            info!("Saved theme state to {}", path.display());
            if let Some(m) = manifest {
                m.push(crate::utils::Artifact::File { path: path.clone() });
            }
        }
        Err(e) => tracing::warn!("Failed to save theme state: {}", e),
    }

    info!("Applied theme '{}' to {}", theme_name, init_path.display());
    Ok(init_path)
}

/// Convert a `Color` to a ratatui `RatColor`.
fn to_rat_color(c: Color) -> RatColor {
    let r = ((c.0 >> 16) & 0xFF) as u8;
    let g = ((c.0 >> 8) & 0xFF) as u8;
    let b = (c.0 & 0xFF) as u8;
    RatColor::Rgb(r, g, b)
}

/// Interactive TUI theme picker using ratatui.
pub fn interactive_select() -> Result<String> {
    let mut terminal = Terminal::new(CrosstermBackend::new(std::io::stdout()))?;
    let names = list_theme_names();
    let mut state = ListState::default();
    state.select(Some(0));

    let result = loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints([Constraint::Min(3), Constraint::Length(3)])
                .split(f.area());

            let header = Paragraph::new(vec![
                Line::from("Sheep-RHEL Theme Selector"),
                Line::from("Navigate with ↑/↓, press Enter to select, q to quit."),
            ])
            .block(Block::default().borders(Borders::NONE))
            .style(Style::default().add_modifier(Modifier::BOLD));
            f.render_widget(header, chunks[0]);

            let items: Vec<ListItem> = names
                .iter()
                .map(|name| {
                    let theme = find_theme(name)
                        .expect("theme name from list_theme_names must exist in THEMES");
                    let preview = format!(
                        "  {:15} | focused: {} | bg: {}",
                        name, theme.window_border, theme.message_bg
                    );
                    let content = Line::from(Span::styled(
                        preview,
                        Style::default().fg(to_rat_color(theme.text_color)),
                    ));
                    ListItem::new(content)
                })
                .collect();

            // Highlight colour matches the *selected* theme's border.
            let highlight_bg = state
                .selected()
                .and_then(|idx| find_theme(names[idx]))
                .map(|t| to_rat_color(t.window_border))
                .unwrap_or(RatColor::Red);

            let list = List::new(items)
                .block(Block::default().title("Themes").borders(Borders::ALL))
                .highlight_style(Style::default().bg(highlight_bg).add_modifier(Modifier::BOLD))
                .highlight_symbol(">> ");
            f.render_stateful_widget(list, chunks[1], &mut state);
        })?;

        if !event::poll(Duration::from_secs(30))? {
            break Err(anyhow::anyhow!("Theme selection timed out after 30 seconds"));
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    break Err(anyhow::anyhow!("Theme selection cancelled"));
                }
                KeyCode::Down => {
                    let next = state
                        .selected()
                        .map(|i| (i + 1).min(names.len() - 1))
                        .unwrap_or(0);
                    state.select(Some(next));
                }
                KeyCode::Up => {
                    let prev = state
                        .selected()
                        .map(|i| i.saturating_sub(1))
                        .unwrap_or(0);
                    state.select(Some(prev));
                }
                KeyCode::Enter => {
                    if let Some(idx) = state.selected() {
                        break Ok(names[idx].to_string());
                    }
                }
                _ => {}
            }
        }
    };

    terminal.clear()?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_to_rrggbb_extracts_rgb() {
        let c = Color::new(0xff, 0xEE, 0x00, 0x00);
        assert_eq!(c.to_rrggbb(), "ee0000");
    }

    #[test]
    fn color_display_includes_prefix() {
        let c = Color::new(0xff, 0x12, 0x34, 0x56);
        assert_eq!(format!("{}", c), "0xff123456");
    }

    #[test]
    fn find_theme_case_insensitive() {
        assert!(find_theme("RHEL-RED").is_some());
        assert!(find_theme("rhel-red").is_some());
        assert!(find_theme("EmBeR-GlOw").is_some());
        assert!(find_theme("nonexistent").is_none());
    }

    #[test]
    fn all_themes_have_unique_names() {
        let names: Vec<_> = list_theme_names();
        let mut deduped = names.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(names.len(), deduped.len(), "theme names must be unique");
    }

    #[test]
    fn theme_state_roundtrips_via_json() {
        let state = ThemeState {
            name: "test-theme".to_string(),
            bg: "1c1c1c".to_string(),
            fg: "ffffff".to_string(),
            sel_bg: "ee0000".to_string(),
            sel_fg: "ffffff".to_string(),
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        let parsed: ThemeState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, state.name);
        assert_eq!(parsed.bg, state.bg);
        assert_eq!(parsed.fg, state.fg);
        assert_eq!(parsed.sel_bg, state.sel_bg);
        assert_eq!(parsed.sel_fg, state.sel_fg);
    }

    #[test]
    fn generate_river_init_contains_theme_name() {
        let script = generate_river_init_script("rhel-red").unwrap();
        assert!(script.contains("Theme: rhel-red"));
        assert!(script.contains("riverctl border-color-focused"));
        assert!(script.contains("sheep-run"));
    }

    #[test]
    fn generate_river_init_rejects_unknown_theme() {
        let result = generate_river_init_script("does-not-exist");
        assert!(result.is_err());
    }

    #[test]
    fn new_themes_present_in_themes_array() {
        assert!(find_theme("crimson-night").is_some());
        assert!(find_theme("rh-silver").is_some());
        assert!(find_theme("ember-glow").is_some());
    }

    #[test]
    fn wallpaper_logic_checks_for_redhat_png() {
        let script = generate_river_init_script("rhel-red").unwrap();
        assert!(
            script.contains("$HOME/Pictures/Wallpaper/redhat.png"),
            "init script must check for redhat.png wallpaper"
        );
    }

    #[test]
    fn wallpaper_uses_swaybg_when_present() {
        let script = generate_river_init_script("rhel-red").unwrap();
        assert!(
            script.contains("swaybg -o '*'"),
            "init script must launch swaybg for all outputs"
        );
        assert!(
            script.contains("-m fill"),
            "init script must use fill mode for wallpaper"
        );
    }

    #[test]
    fn wallpaper_fallback_to_background_color() {
        let script = generate_river_init_script("rhel-red").unwrap();
        assert!(
            script.contains("else"),
            "init script must have else branch for wallpaper fallback"
        );
        assert!(
            script.contains("riverctl background-color"),
            "init script must fall back to background-color when wallpaper is missing"
        );
    }
}
