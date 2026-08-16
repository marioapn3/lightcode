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
    pub blue: Color,
    pub bg: Color,
    pub bg_alt: Color,
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
            blue: Color::Rgb(59, 130, 246),
            bg: Color::Rgb(2, 7, 19),
            bg_alt: Color::Rgb(2, 6, 23),
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
            blue: Color::Rgb(94, 129, 172),
            bg: Color::Rgb(46, 52, 64),
            bg_alt: Color::Rgb(59, 66, 82),
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
            blue: Color::Rgb(98, 114, 164),
            bg: Color::Rgb(40, 42, 54),
            bg_alt: Color::Rgb(68, 71, 90),
        }
    }

    /// Ocean: deep blue water.
    pub const fn ocean() -> Theme {
        Theme {
            name: "ocean",
            text: Color::Rgb(226, 240, 255),
            dim: Color::Rgb(100, 130, 160),
            accent: Color::Rgb(56, 189, 248),
            success: Color::Rgb(74, 222, 128),
            error: Color::Rgb(248, 113, 113),
            running: Color::Rgb(251, 191, 36),
            reasoning: Color::Rgb(147, 197, 253),
            border: Color::Rgb(51, 85, 120),
            selection_fg: Color::Rgb(8, 47, 73),
            selection_bg: Color::Rgb(125, 211, 252),
            hover_bg: Color::Rgb(15, 60, 90),
            code: Color::Rgb(147, 197, 253),
            heading: Color::Rgb(240, 249, 255),
            blue: Color::Rgb(56, 189, 248),
            bg: Color::Rgb(2, 16, 30),
            bg_alt: Color::Rgb(8, 35, 60),
        }
    }

    /// Sunset: warm orange/pink dusk.
    pub const fn sunset() -> Theme {
        Theme {
            name: "sunset",
            text: Color::Rgb(255, 240, 235),
            dim: Color::Rgb(180, 130, 120),
            accent: Color::Rgb(251, 146, 60),
            success: Color::Rgb(74, 222, 128),
            error: Color::Rgb(248, 113, 113),
            running: Color::Rgb(244, 114, 182),
            reasoning: Color::Rgb(253, 164, 175),
            border: Color::Rgb(150, 90, 70),
            selection_fg: Color::Rgb(70, 25, 10),
            selection_bg: Color::Rgb(253, 186, 116),
            hover_bg: Color::Rgb(90, 40, 25),
            code: Color::Rgb(254, 205, 211),
            heading: Color::Rgb(255, 237, 213),
            blue: Color::Rgb(251, 146, 60),
            bg: Color::Rgb(30, 12, 20),
            bg_alt: Color::Rgb(60, 28, 25),
        }
    }

    /// Forest: green canopies.
    pub const fn forest() -> Theme {
        Theme {
            name: "forest",
            text: Color::Rgb(236, 253, 245),
            dim: Color::Rgb(110, 160, 130),
            accent: Color::Rgb(74, 222, 128),
            success: Color::Rgb(163, 230, 53),
            error: Color::Rgb(248, 113, 113),
            running: Color::Rgb(250, 204, 21),
            reasoning: Color::Rgb(134, 239, 172),
            border: Color::Rgb(70, 120, 90),
            selection_fg: Color::Rgb(20, 60, 35),
            selection_bg: Color::Rgb(134, 239, 172),
            hover_bg: Color::Rgb(30, 70, 45),
            code: Color::Rgb(163, 230, 53),
            heading: Color::Rgb(240, 253, 244),
            blue: Color::Rgb(74, 222, 128),
            bg: Color::Rgb(6, 24, 14),
            bg_alt: Color::Rgb(15, 45, 26),
        }
    }

    /// Grape: purple/violet.
    pub const fn grape() -> Theme {
        Theme {
            name: "grape",
            text: Color::Rgb(245, 240, 255),
            dim: Color::Rgb(150, 130, 190),
            accent: Color::Rgb(192, 132, 252),
            success: Color::Rgb(74, 222, 128),
            error: Color::Rgb(251, 113, 133),
            running: Color::Rgb(251, 191, 36),
            reasoning: Color::Rgb(216, 180, 254),
            border: Color::Rgb(120, 80, 170),
            selection_fg: Color::Rgb(50, 20, 90),
            selection_bg: Color::Rgb(216, 180, 254),
            hover_bg: Color::Rgb(55, 30, 90),
            code: Color::Rgb(196, 181, 253),
            heading: Color::Rgb(250, 245, 255),
            blue: Color::Rgb(192, 132, 252),
            bg: Color::Rgb(22, 8, 40),
            bg_alt: Color::Rgb(45, 20, 75),
        }
    }

    /// Amber: warm honey/gold.
    pub const fn amber() -> Theme {
        Theme {
            name: "amber",
            text: Color::Rgb(255, 250, 235),
            dim: Color::Rgb(185, 160, 110),
            accent: Color::Rgb(251, 191, 36),
            success: Color::Rgb(74, 222, 128),
            error: Color::Rgb(248, 113, 113),
            running: Color::Rgb(251, 146, 60),
            reasoning: Color::Rgb(253, 224, 71),
            border: Color::Rgb(150, 120, 50),
            selection_fg: Color::Rgb(70, 50, 5),
            selection_bg: Color::Rgb(253, 224, 71),
            hover_bg: Color::Rgb(85, 65, 20),
            code: Color::Rgb(254, 240, 138),
            heading: Color::Rgb(255, 251, 235),
            blue: Color::Rgb(251, 191, 36),
            bg: Color::Rgb(26, 18, 4),
            bg_alt: Color::Rgb(55, 42, 15),
        }
    }

    /// Slate: cool monochrome steel.
    pub const fn slate() -> Theme {
        Theme {
            name: "slate",
            text: Color::Rgb(241, 245, 249),
            dim: Color::Rgb(120, 140, 160),
            accent: Color::Rgb(148, 163, 184),
            success: Color::Rgb(94, 234, 212),
            error: Color::Rgb(248, 113, 113),
            running: Color::Rgb(226, 232, 240),
            reasoning: Color::Rgb(203, 213, 225),
            border: Color::Rgb(71, 85, 105),
            selection_fg: Color::Rgb(15, 23, 42),
            selection_bg: Color::Rgb(148, 163, 184),
            hover_bg: Color::Rgb(51, 65, 85),
            code: Color::Rgb(125, 211, 252),
            heading: Color::Rgb(248, 250, 252),
            blue: Color::Rgb(148, 163, 184),
            bg: Color::Rgb(2, 7, 19),
            bg_alt: Color::Rgb(30, 41, 59),
        }
    }

    /// Rose: soft pink.
    pub const fn rose() -> Theme {
        Theme {
            name: "rose",
            text: Color::Rgb(255, 241, 242),
            dim: Color::Rgb(190, 140, 145),
            accent: Color::Rgb(251, 113, 133),
            success: Color::Rgb(74, 222, 128),
            error: Color::Rgb(248, 113, 113),
            running: Color::Rgb(253, 164, 175),
            reasoning: Color::Rgb(244, 114, 182),
            border: Color::Rgb(160, 90, 100),
            selection_fg: Color::Rgb(80, 20, 30),
            selection_bg: Color::Rgb(253, 164, 175),
            hover_bg: Color::Rgb(100, 40, 50),
            code: Color::Rgb(254, 205, 211),
            heading: Color::Rgb(255, 241, 242),
            blue: Color::Rgb(251, 113, 133),
            bg: Color::Rgb(30, 8, 14),
            bg_alt: Color::Rgb(60, 22, 32),
        }
    }

    /// Midnight: deep indigo night.
    pub const fn midnight() -> Theme {
        Theme {
            name: "midnight",
            text: Color::Rgb(238, 242, 255),
            dim: Color::Rgb(130, 140, 200),
            accent: Color::Rgb(129, 140, 248),
            success: Color::Rgb(74, 222, 128),
            error: Color::Rgb(248, 113, 113),
            running: Color::Rgb(250, 204, 21),
            reasoning: Color::Rgb(165, 180, 252),
            border: Color::Rgb(80, 90, 160),
            selection_fg: Color::Rgb(30, 30, 80),
            selection_bg: Color::Rgb(165, 180, 252),
            hover_bg: Color::Rgb(45, 50, 110),
            code: Color::Rgb(199, 210, 254),
            heading: Color::Rgb(238, 242, 255),
            blue: Color::Rgb(129, 140, 248),
            bg: Color::Rgb(8, 10, 34),
            bg_alt: Color::Rgb(22, 26, 70),
        }
    }

    /// Emerald: bright jewel green.
    pub const fn emerald() -> Theme {
        Theme {
            name: "emerald",
            text: Color::Rgb(236, 253, 245),
            dim: Color::Rgb(110, 175, 145),
            accent: Color::Rgb(52, 211, 153),
            success: Color::Rgb(134, 239, 172),
            error: Color::Rgb(252, 165, 165),
            running: Color::Rgb(251, 191, 36),
            reasoning: Color::Rgb(110, 231, 183),
            border: Color::Rgb(50, 130, 100),
            selection_fg: Color::Rgb(5, 60, 40),
            selection_bg: Color::Rgb(110, 231, 183),
            hover_bg: Color::Rgb(20, 70, 50),
            code: Color::Rgb(167, 243, 208),
            heading: Color::Rgb(236, 253, 245),
            blue: Color::Rgb(52, 211, 153),
            bg: Color::Rgb(4, 24, 18),
            bg_alt: Color::Rgb(12, 48, 36),
        }
    }

    /// Candy: playful multi-color.
    pub const fn candy() -> Theme {
        Theme {
            name: "candy",
            text: Color::Rgb(255, 240, 250),
            dim: Color::Rgb(200, 150, 180),
            accent: Color::Rgb(232, 121, 249),
            success: Color::Rgb(74, 222, 128),
            error: Color::Rgb(249, 115, 22),
            running: Color::Rgb(251, 146, 60),
            reasoning: Color::Rgb(244, 114, 182),
            border: Color::Rgb(180, 110, 150),
            selection_fg: Color::Rgb(70, 20, 60),
            selection_bg: Color::Rgb(249, 168, 212),
            hover_bg: Color::Rgb(100, 45, 85),
            code: Color::Rgb(253, 164, 175),
            heading: Color::Rgb(255, 241, 242),
            blue: Color::Rgb(129, 140, 248),
            bg: Color::Rgb(30, 10, 28),
            bg_alt: Color::Rgb(60, 28, 55),
        }
    }

    /// The 3 built-in themes.
    pub fn all() -> Vec<Theme> {
        vec![
            Theme::default_theme(),
            Theme::nord(),
            Theme::dracula(),
            Theme::ocean(),
            Theme::sunset(),
            Theme::forest(),
            Theme::grape(),
            Theme::amber(),
            Theme::slate(),
            Theme::rose(),
            Theme::midnight(),
            Theme::emerald(),
            Theme::candy(),
        ]
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
        for t in Theme::all() {
            assert_eq!(Theme::by_name(t.name).unwrap().name, t.name);
        }
        assert!(Theme::by_name("nope").is_none());
    }

    #[test]
    fn palettes_are_distinct() {
        let t = Theme::all();
        assert_eq!(t.len(), 13);
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
