use ratatui::style::Color;
use std::sync::RwLock;

/// Semantic color roles for the TUI. The renderer maps every hardcoded color
/// to a role so themes can re-skin the whole UI without touching draw code.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub name: &'static str,
    pub text: Color,
    pub dim: Color,
    pub accent: Color,
    pub success: Color,
    pub error: Color,
    pub running: Color,
    pub reasoning: Color,
    pub border: Color,
    pub selection_fg: Color,
    pub selection_bg: Color,
    pub hover_bg: Color,
    pub code: Color,
    pub heading: Color,
    pub bg: Color,
}

static CURRENT: RwLock<Theme> = RwLock::new(Theme::default_theme());

impl Theme {
    /// The built-in LightCode look (current colors).
    pub const fn default_theme() -> Theme {
        Theme {
            name: "default",
            text: Color::White,
            dim: Color::DarkGray,
            accent: Color::Cyan,
            success: Color::Green,
            error: Color::Red,
            running: Color::Yellow,
            reasoning: Color::Magenta,
            border: Color::DarkGray,
            selection_fg: Color::Black,
            selection_bg: Color::White,
            hover_bg: Color::Indexed(236),
            code: Color::Yellow,
            heading: Color::White,
            bg: Color::Rgb(30, 41, 59),
        }
    }

    /// Nord: cool blue-gray palette.
    pub const fn nord() -> Theme {
        Theme {
            name: "nord",
            text: Color::Rgb(216, 222, 233),
            dim: Color::Rgb(76, 86, 106),
            accent: Color::Rgb(136, 192, 208),
            success: Color::Rgb(163, 190, 140),
            error: Color::Rgb(191, 97, 106),
            running: Color::Rgb(235, 203, 139),
            reasoning: Color::Rgb(180, 142, 173),
            border: Color::Rgb(59, 66, 82),
            selection_fg: Color::Rgb(46, 52, 64),
            selection_bg: Color::Rgb(129, 161, 193),
            hover_bg: Color::Rgb(59, 66, 82),
            code: Color::Rgb(143, 188, 187),
            heading: Color::Rgb(216, 222, 233),
            bg: Color::Rgb(46, 52, 64),
        }
    }

    /// Dracula: high-contrast dark palette.
    pub const fn dracula() -> Theme {
        Theme {
            name: "dracula",
            text: Color::Rgb(248, 248, 242),
            dim: Color::Rgb(98, 114, 164),
            accent: Color::Rgb(139, 233, 253),
            success: Color::Rgb(80, 250, 123),
            error: Color::Rgb(255, 85, 85),
            running: Color::Rgb(241, 250, 140),
            reasoning: Color::Rgb(189, 147, 249),
            border: Color::Rgb(68, 71, 90),
            selection_fg: Color::Rgb(40, 42, 54),
            selection_bg: Color::Rgb(68, 71, 90),
            hover_bg: Color::Rgb(68, 71, 90),
            code: Color::Rgb(139, 233, 253),
            heading: Color::Rgb(248, 248, 242),
            bg: Color::Rgb(40, 42, 54),
        }
    }

    /// The 3 built-in themes.
    pub fn all() -> Vec<Theme> {
        vec![Theme::default_theme(), Theme::nord(), Theme::dracula()]
    }

    pub fn by_name(name: &str) -> Option<Theme> {
        Self::all().into_iter().find(|t| t.name == name)
    }

    /// The currently active theme (a cheap copy).
    pub fn current() -> Theme {
        *CURRENT.read().expect("theme lock")
    }

    /// Activate a theme globally.
    pub fn set(&self) {
        *CURRENT.write().expect("theme lock") = *self;
    }
}

/// Persisted theme selection, stored next to the sessions dir.
fn state_path() -> std::path::PathBuf {
    crate::session::storage::sessions_dir().join("ui.json")
}

pub fn load() -> Option<Theme> {
    let text = std::fs::read_to_string(state_path()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = v.get("theme")?.as_str()?;
    Theme::by_name(name)
}

pub fn save(theme: &Theme) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        path,
        serde_json::to_string(&serde_json::json!({ "theme": theme.name })).unwrap_or_default(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_resolve_by_name() {
        assert_eq!(Theme::by_name("default").unwrap().name, "default");
        assert_eq!(Theme::by_name("nord").unwrap().name, "nord");
        assert_eq!(Theme::by_name("dracula").unwrap().name, "dracula");
        assert!(Theme::by_name("nope").is_none());
    }

    #[test]
    fn palettes_are_distinct() {
        let t = Theme::all();
        assert_eq!(t.len(), 3);
        // nord and dracula differ from default on the accent color.
        assert_ne!(Theme::nord().accent, Theme::default_theme().accent);
        assert_ne!(Theme::dracula().accent, Theme::default_theme().accent);
    }

    #[test]
    fn set_and_current_roundtrip() {
        Theme::set(&Theme::nord());
        assert_eq!(Theme::current().name, "nord");
        Theme::set(&Theme::default_theme());
        assert_eq!(Theme::current().name, "default");
    }
}
