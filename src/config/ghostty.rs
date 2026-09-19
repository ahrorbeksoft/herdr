//! Ghostty theme discovery, parsing, and config helpers.
//!
//! Herdr can derive its palette from the theme Ghostty is currently using:
//! `theme.name = "ghostty"` follows Ghostty's configured `theme` (including
//! `dark:X,light:Y` splits), and `theme.name = "ghostty:<name>"` pins a
//! specific Ghostty theme by name.

use std::path::PathBuf;

/// Prefix selecting a specific Ghostty theme in `theme.name` et al.
pub const GHOSTTY_THEME_PREFIX: &str = "ghostty:";

/// Override used by tests and advanced setups to point at a specific
/// directory containing Ghostty theme files.
pub const GHOSTTY_THEMES_DIR_ENV: &str = "HERDR_GHOSTTY_THEMES_DIR";
/// Override for the Ghostty configuration file path (mirrors the
/// `GHOSTTY_CONFIG` convention used by the theme-sync helper script).
pub const GHOSTTY_CONFIG_ENV: &str = "GHOSTTY_CONFIG";

/// Subset of a Ghostty theme file relevant to palette synthesis.
///
/// Colors are stored verbatim (hex strings, typically `#rrggbb`) and parsed
/// later through [`crate::config::parse_color`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GhosttyThemeSpec {
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub cursor_color: Option<String>,
    pub selection_background: Option<String>,
    pub selection_foreground: Option<String>,
    /// ANSI colors 0-15 (`palette = N=#hex` lines).
    pub palette: [Option<String>; 16],
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value)
}

/// Parse a Ghostty theme file. Unknown keys are ignored; colors are kept as
/// raw strings so `parse_color` handles validation later.
pub fn parse_ghostty_theme(content: &str) -> GhosttyThemeSpec {
    let mut spec = GhosttyThemeSpec::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = unquote(value);
        match key {
            "background" => spec.background = Some(value.to_owned()),
            "foreground" => spec.foreground = Some(value.to_owned()),
            "cursor-color" => spec.cursor_color = Some(value.to_owned()),
            "selection-background" => spec.selection_background = Some(value.to_owned()),
            "selection-foreground" => spec.selection_foreground = Some(value.to_owned()),
            "palette" => {
                if let Some((index, color)) = value.split_once('=') {
                    if let Ok(index) = index.trim().parse::<usize>() {
                        if index < spec.palette.len() {
                            spec.palette[index] = Some(unquote(color).to_owned());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    spec
}

fn config_home() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(xdg);
    }
    home_dir().join(".config")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Ghostty theme directories, in precedence order: user themes first (they
/// override bundled themes of the same name in Ghostty), then resources.
pub fn ghostty_theme_dirs() -> Vec<PathBuf> {
    if let Some(custom) = std::env::var_os(GHOSTTY_THEMES_DIR_ENV).filter(|v| !v.is_empty()) {
        return vec![PathBuf::from(custom)];
    }
    let mut dirs = vec![config_home().join("ghostty").join("themes")];
    if let Some(resources) = std::env::var_os("GHOSTTY_RESOURCES_DIR").filter(|v| !v.is_empty()) {
        dirs.push(PathBuf::from(resources).join("themes"));
    }
    for extra in [
        "/Applications/Ghostty.app/Contents/Resources/ghostty/themes",
        "/usr/local/share/ghostty/themes",
        "/usr/share/ghostty/themes",
        "~/.local/share/ghostty/themes",
    ] {
        let dir = if let Some(rest) = extra.strip_prefix("~/") {
            home_dir().join(rest)
        } else {
            PathBuf::from(extra)
        };
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Sorted, deduplicated list of available Ghostty theme names.
pub fn ghostty_theme_names() -> Vec<String> {
    let mut names = Vec::new();
    for dir in ghostty_theme_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                names.push(name.to_owned());
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn ghostty_theme_path(name: &str) -> Option<PathBuf> {
    ghostty_theme_dirs()
        .into_iter()
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

/// Load a Ghostty theme by its display name (the file name in a theme dir).
pub fn load_ghostty_theme(name: &str) -> Option<GhosttyThemeSpec> {
    let path = ghostty_theme_path(name)?;
    let content = std::fs::read_to_string(&path).ok()?;
    Some(parse_ghostty_theme(&content))
}

/// Path to Ghostty's configuration file (`theme =` lives here).
pub fn ghostty_config_path() -> PathBuf {
    if let Some(custom) = std::env::var_os(GHOSTTY_CONFIG_ENV).filter(|v| !v.is_empty()) {
        return PathBuf::from(custom);
    }
    config_home().join("ghostty").join("config")
}

/// The value of Ghostty's `theme` key: a single theme or a dark/light pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhosttyThemeSetting {
    Single(String),
    Split { dark: String, light: String },
}

/// Parse a `theme` value such as `Gruvbox Dark` or `dark:A,light:B`.
pub fn parse_ghostty_theme_setting(value: &str) -> Option<GhosttyThemeSetting> {
    let mut dark = None;
    let mut light = None;
    let mut plain = Vec::new();
    for part in value.split(',') {
        let part = part.trim();
        if let Some(name) = part
            .strip_prefix("dark:")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            dark = Some(name.to_owned());
        } else if let Some(name) = part
            .strip_prefix("light:")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            light = Some(name.to_owned());
        } else if !part.is_empty() {
            plain.push(part.to_owned());
        }
    }
    match (dark, light) {
        (Some(dark), Some(light)) => Some(GhosttyThemeSetting::Split { dark, light }),
        (Some(dark), None) if plain.is_empty() => Some(GhosttyThemeSetting::Single(dark)),
        (None, Some(light)) if plain.is_empty() => Some(GhosttyThemeSetting::Single(light)),
        (None, None) if !plain.is_empty() => Some(GhosttyThemeSetting::Single(plain.join(", "))),
        _ => plain
            .first()
            .map(|name| GhosttyThemeSetting::Single(name.clone())),
    }
}

/// The verbatim `theme` value from Ghostty config content — the last
/// un-commented `theme =` line, matching Ghostty's "later overrides"
/// semantics for repeated scalar keys.
pub fn ghostty_config_theme_value(content: &str) -> Option<String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with('#') {
                return None;
            }
            line.strip_prefix("theme")
                .and_then(|rest| rest.trim_start().strip_prefix('='))
                .map(|value| unquote(value).to_owned())
        })
        .last()
}

/// The effective `theme` setting from Ghostty config content.
pub fn ghostty_config_theme_setting(content: &str) -> Option<GhosttyThemeSetting> {
    ghostty_config_theme_value(content).and_then(|value| parse_ghostty_theme_setting(&value))
}

/// Rewrite the `theme` line of Ghostty config content.
///
/// - `Some(value)` replaces the first active `theme =` line, or appends one.
/// - `None` comments out active `theme =` lines.
/// Other content is preserved byte-for-byte.
pub fn rewrite_ghostty_config_theme(content: &str, value: Option<&str>) -> String {
    let mut lines: Vec<String> = content.lines().map(str::to_owned).collect();
    let mut wrote = false;
    for line in &mut lines {
        let trimmed = line.trim_start();
        let is_theme = !trimmed.starts_with('#')
            && trimmed
                .strip_prefix("theme")
                .is_some_and(|rest| rest.trim_start().starts_with('='));
        if !is_theme {
            continue;
        }
        match (value, wrote) {
            (Some(value), false) => {
                *line = format!("theme = {value}");
                wrote = true;
            }
            _ => *line = format!("# {line}"),
        }
    }
    if let Some(value) = value.filter(|_| !wrote) {
        if !lines.last().is_some_and(|line| line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(format!("theme = {value}"));
    }
    let mut out = lines.join("\n");
    if content.ends_with('\n') || (value.is_some() && !wrote) {
        out.push('\n');
    }
    out
}

/// Interpret a `theme.name`-style value as a Ghostty theme reference.
///
/// - `"ghostty"` → `Some(None)` — follow Ghostty's own configured theme.
/// - `"ghostty:<name>"` → `Some(Some(name))` — pin that Ghostty theme.
/// - anything else → `None`.
pub fn ghostty_theme_ref(name: &str) -> Option<Option<String>> {
    let name = name.trim();
    if name.eq_ignore_ascii_case("ghostty") {
        return Some(None);
    }
    name.strip_prefix(GHOSTTY_THEME_PREFIX)
        .map(str::trim)
        .filter(|inner| !inner.is_empty())
        .map(|inner| Some(inner.to_owned()))
}

/// Map a built-in herdr theme to the Ghostty theme that matches it, so
/// built-in selections can also be previewed/applied on the terminal side.
pub fn builtin_to_ghostty_name(name: &str) -> Option<&'static str> {
    match super::canonical_theme_name(name)? {
        "terminal" => None,
        "catppuccin" => Some("Catppuccin Mocha"),
        "catppuccin-latte" => Some("Catppuccin Latte"),
        "tokyo-night" => Some("TokyoNight Storm"),
        "tokyo-night-day" => Some("TokyoNight Day"),
        "dracula" => Some("Dracula"),
        "nord" => Some("Nord"),
        "gruvbox" => Some("Gruvbox Dark"),
        "gruvbox-light" => Some("Gruvbox Light"),
        "one-dark" => Some("Atom One Dark"),
        "one-light" => Some("Atom One Light"),
        "solarized" => Some("iTerm2 Solarized Dark"),
        "solarized-light" => Some("iTerm2 Solarized Light"),
        "kanagawa" => Some("Kanagawa Wave"),
        "kanagawa-lotus" => Some("Kanagawa Lotus"),
        "rose-pine" => Some("Rose Pine"),
        "rose-pine-dawn" => Some("Rose Pine Dawn"),
        "vesper" => Some("Vesper"),
        _ => None,
    }
}

/// Resolve the Ghostty theme a herdr theme name should preview/apply as.
///
/// Returns `Some(name)` when a concrete Ghostty theme should be written,
/// `None` when the terminal theme should be left alone.
pub fn ghostty_name_for_herdr_theme(name: &str) -> Option<String> {
    match ghostty_theme_ref(name) {
        Some(Some(pinned)) => Some(pinned),
        Some(None) | None => builtin_to_ghostty_name(name).map(str::to_owned),
    }
}

fn spec_color(value: Option<&String>) -> Option<crate::terminal_theme::RgbColor> {
    match super::parse_color(value?) {
        ratatui::style::Color::Rgb(r, g, b) => Some(crate::terminal_theme::RgbColor { r, g, b }),
        _ => None,
    }
}

/// OSC escape sequence that recolors the host terminal with a Ghostty
/// theme's palette — the instant preview path; no config write involved.
/// Fields missing from the theme file are left untouched.
pub fn ghostty_preview_sequence(spec: &GhosttyThemeSpec) -> String {
    use crate::terminal_theme::*;
    let mut sequence = String::new();
    if let Some(color) = spec_color(spec.background.as_ref()) {
        sequence.push_str(&osc_set_default_color_sequence(
            DefaultColorKind::Background,
            color,
        ));
    }
    if let Some(color) = spec_color(spec.foreground.as_ref()) {
        sequence.push_str(&osc_set_default_color_sequence(
            DefaultColorKind::Foreground,
            color,
        ));
    }
    if let Some(color) = spec_color(spec.cursor_color.as_ref()) {
        sequence.push_str(&osc_set_cursor_color_sequence(color));
    }
    if let Some(color) = spec_color(spec.selection_background.as_ref()) {
        sequence.push_str(&osc_set_selection_color_sequence(false, color));
    }
    if let Some(color) = spec_color(spec.selection_foreground.as_ref()) {
        sequence.push_str(&osc_set_selection_color_sequence(true, color));
    }
    for (index, value) in spec.palette.iter().enumerate() {
        if let Some(color) = spec_color(value.as_ref()) {
            sequence.push_str(&osc_set_palette_color_sequence(index as u8, color));
        }
    }
    sequence
}

/// OSC escape sequence restoring every terminal color the preview path can
/// touch back to the terminal's configured theme.
pub fn ghostty_preview_reset_sequence() -> String {
    use crate::terminal_theme::*;
    let mut sequence = String::new();
    sequence.push_str(osc_reset_default_color_sequence(
        DefaultColorKind::Background,
    ));
    sequence.push_str(osc_reset_default_color_sequence(
        DefaultColorKind::Foreground,
    ));
    sequence.push_str(OSC_RESET_CURSOR_COLOR_SEQUENCE);
    sequence.push_str(OSC_RESET_SELECTION_BACKGROUND_SEQUENCE);
    sequence.push_str(OSC_RESET_SELECTION_FOREGROUND_SEQUENCE);
    sequence.push_str(OSC_RESET_PALETTE_COLORS_SEQUENCE);
    sequence
}

/// The config content to write when Ghostty's configured theme should
/// become `desired`, or `None` when it already matches.
pub fn ghostty_config_sync_update(content: &str, desired: &str) -> Option<String> {
    if ghostty_config_theme_value(content).as_deref() == Some(desired) {
        return None;
    }
    let updated = rewrite_ghostty_config_theme(content, Some(desired));
    (updated != content).then_some(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# comment
palette = 0=#26233a
palette = 1=#eb6f92
palette = 4=#9ccfd8
palette = 11=#f6c177
palette = 15=#e0def4
background = #191724
foreground = #e0def4
cursor-color = #e0def4
selection-background = #403d52
selection-foreground = #e0def4
unknown-key = ignored
"#;

    #[test]
    fn parses_theme_spec() {
        let spec = parse_ghostty_theme(SAMPLE);
        assert_eq!(spec.background.as_deref(), Some("#191724"));
        assert_eq!(spec.foreground.as_deref(), Some("#e0def4"));
        assert_eq!(spec.cursor_color.as_deref(), Some("#e0def4"));
        assert_eq!(spec.selection_background.as_deref(), Some("#403d52"));
        assert_eq!(spec.palette[0].as_deref(), Some("#26233a"));
        assert_eq!(spec.palette[1].as_deref(), Some("#eb6f92"));
        assert_eq!(spec.palette[4].as_deref(), Some("#9ccfd8"));
        assert_eq!(spec.palette[15].as_deref(), Some("#e0def4"));
        assert!(spec.palette[2].is_none());
    }

    #[test]
    fn parses_theme_settings() {
        assert_eq!(
            parse_ghostty_theme_setting("Gruvbox Dark"),
            Some(GhosttyThemeSetting::Single("Gruvbox Dark".into()))
        );
        assert_eq!(
            parse_ghostty_theme_setting("dark:Gruvbox Dark,light:Gruvbox Light"),
            Some(GhosttyThemeSetting::Split {
                dark: "Gruvbox Dark".into(),
                light: "Gruvbox Light".into(),
            })
        );
        assert_eq!(
            parse_ghostty_theme_setting("light:L,dark:D"),
            Some(GhosttyThemeSetting::Split {
                dark: "D".into(),
                light: "L".into(),
            })
        );
        assert_eq!(parse_ghostty_theme_setting(""), None);
    }

    #[test]
    fn config_theme_setting_uses_last_active_line() {
        let content = "# theme = Old\ntheme = First\ntheme = \"dark:D,light:L\"\n";
        assert_eq!(
            ghostty_config_theme_setting(content),
            Some(GhosttyThemeSetting::Split {
                dark: "D".into(),
                light: "L".into(),
            })
        );
    }

    #[test]
    fn rewrite_replaces_active_line_and_comments_rest() {
        let content = "foo = 1\ntheme = Old\ntheme = Older\n";
        let out = rewrite_ghostty_config_theme(content, Some("New"));
        assert_eq!(out, "foo = 1\ntheme = New\n# theme = Older\n");
    }

    #[test]
    fn rewrite_appends_when_missing() {
        let out = rewrite_ghostty_config_theme("foo = 1\n", Some("New"));
        assert_eq!(out, "foo = 1\n\ntheme = New\n");
    }

    #[test]
    fn rewrite_none_comments_out() {
        let out = rewrite_ghostty_config_theme("theme = Old\nfoo = 1\n", None);
        assert_eq!(out, "# theme = Old\nfoo = 1\n");
    }

    #[test]
    fn theme_ref_parses() {
        assert_eq!(ghostty_theme_ref("ghostty"), Some(None));
        assert_eq!(
            ghostty_theme_ref("ghostty:Gruvbox Dark"),
            Some(Some("Gruvbox Dark".into()))
        );
        assert_eq!(ghostty_theme_ref("ghostty:"), None);
        assert_eq!(ghostty_theme_ref("dracula"), None);
    }

    #[test]
    fn builtin_maps_to_ghostty() {
        assert_eq!(builtin_to_ghostty_name("rose-pine"), Some("Rose Pine"));
        assert_eq!(
            builtin_to_ghostty_name("tokyonight"),
            Some("TokyoNight Storm")
        );
        assert_eq!(builtin_to_ghostty_name("terminal"), None);
    }

    #[test]
    fn herdr_theme_to_ghostty_name() {
        assert_eq!(
            ghostty_name_for_herdr_theme("ghostty:Gruvbox Dark").as_deref(),
            Some("Gruvbox Dark")
        );
        assert_eq!(
            ghostty_name_for_herdr_theme("nord").as_deref(),
            Some("Nord")
        );
        assert_eq!(ghostty_name_for_herdr_theme("terminal"), None);
    }

    #[test]
    fn preview_sequence_sets_spec_colors() {
        let sequence = ghostty_preview_sequence(&parse_ghostty_theme(SAMPLE));
        assert!(
            sequence.contains("\x1b]11;rgb:19/17/24\x1b\\"),
            "{sequence}"
        );
        assert!(
            sequence.contains("\x1b]10;rgb:e0/de/f4\x1b\\"),
            "{sequence}"
        );
        assert!(
            sequence.contains("\x1b]12;rgb:e0/de/f4\x1b\\"),
            "{sequence}"
        );
        assert!(
            sequence.contains("\x1b]17;rgb:40/3d/52\x1b\\"),
            "{sequence}"
        );
        assert!(
            sequence.contains("\x1b]19;rgb:e0/de/f4\x1b\\"),
            "{sequence}"
        );
        assert!(
            sequence.contains("\x1b]4;0;rgb:26/23/3a\x1b\\"),
            "{sequence}"
        );
        assert!(
            sequence.contains("\x1b]4;11;rgb:f6/c1/77\x1b\\"),
            "{sequence}"
        );
        // Palette slots absent from the file are not emitted.
        assert!(!sequence.contains("\x1b]4;2;"), "{sequence}");
    }

    #[test]
    fn preview_reset_covers_every_set_command() {
        let sequence = ghostty_preview_reset_sequence();
        for expected in [
            "\x1b]111", "\x1b]110", "\x1b]112", "\x1b]117", "\x1b]119", "\x1b]104",
        ] {
            assert!(sequence.contains(expected), "{sequence}");
        }
    }

    #[test]
    fn sync_update_none_when_already_matching() {
        assert_eq!(ghostty_config_sync_update("theme = Nord\n", "Nord"), None);
        assert_eq!(
            ghostty_config_sync_update("theme = \"dark:D,light:L\"\n", "dark:D,light:L"),
            None
        );
    }

    #[test]
    fn sync_update_rewrites_different_theme() {
        let updated = ghostty_config_sync_update("theme = Old\nother = 1\n", "New").unwrap();
        assert_eq!(updated, "theme = New\nother = 1\n");
    }

    #[test]
    fn theme_dir_override_and_loading() {
        let _guard = crate::config::test_config_env_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let dir = std::env::temp_dir().join(format!("herdr-ghostty-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Test Theme"), SAMPLE).unwrap();
        std::fs::write(dir.join("Second"), SAMPLE).unwrap();

        std::env::set_var(GHOSTTY_THEMES_DIR_ENV, &dir);
        assert_eq!(ghostty_theme_names(), ["Second", "Test Theme"]);
        let spec = load_ghostty_theme("Test Theme").unwrap();
        assert_eq!(spec.background.as_deref(), Some("#191724"));
        assert!(load_ghostty_theme("Missing").is_none());

        std::env::remove_var(GHOSTTY_THEMES_DIR_ENV);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
