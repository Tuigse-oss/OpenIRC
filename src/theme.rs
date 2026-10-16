//! The window colors. The Classic scheme is the default.
//!
//! `install` makes the theme current for the UI thread. The window reads it
//! back with `current` while painting.

use std::cell::Cell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn parse(hex: &str) -> Option<Self> {
        let hex = hex.trim().trim_start_matches('#');
        if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(hex, 16).ok()?;
        Some(Self(
            ((value >> 16) & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            (value & 0xff) as u8,
        ))
    }

    pub fn encode(self) -> String {
        format!("{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub face: Rgb,
    pub paper: Rgb,
    pub ink: Rgb,
    pub shadow: Rgb,
    pub hilite: Rgb,
    pub attention: Rgb,
}

impl Theme {
    pub fn classic() -> Self {
        Self {
            face: Rgb(212, 208, 200),
            paper: Rgb(255, 255, 255),
            ink: Rgb(0, 0, 0),
            shadow: Rgb(128, 128, 128),
            hilite: Rgb(10, 36, 106),
            attention: Rgb(128, 0, 0),
        }
    }

    /// The built-in color schemes, by name. Classic is the default.
    pub fn schemes() -> [(&'static str, Theme); 4] {
        [
            (crate::i18n::tr_key("Classic"), Theme::classic()),
            (
                crate::i18n::tr_key("Daylight"),
                Theme {
                    face: Rgb(236, 239, 244),
                    paper: Rgb(255, 255, 255),
                    ink: Rgb(30, 34, 42),
                    shadow: Rgb(186, 193, 204),
                    hilite: Rgb(38, 99, 210),
                    attention: Rgb(190, 40, 40),
                },
            ),
            (
                crate::i18n::tr_key("Midnight"),
                Theme {
                    face: Rgb(36, 40, 49),
                    paper: Rgb(23, 26, 32),
                    ink: Rgb(220, 224, 231),
                    shadow: Rgb(66, 72, 85),
                    hilite: Rgb(66, 125, 235),
                    attention: Rgb(240, 120, 95),
                },
            ),
            (
                crate::i18n::tr_key("High contrast"),
                Theme {
                    face: Rgb(16, 16, 16),
                    paper: Rgb(0, 0, 0),
                    ink: Rgb(255, 255, 255),
                    shadow: Rgb(190, 190, 190),
                    hilite: Rgb(255, 214, 0),
                    attention: Rgb(255, 96, 96),
                },
            ),
        ]
    }

    pub fn set(&mut self, name: &str, color: Rgb) {
        match name {
            "face" => self.face = color,
            "paper" => self.paper = color,
            "ink" => self.ink = color,
            "shadow" => self.shadow = color,
            "hilite" => self.hilite = color,
            "attention" => self.attention = color,
            _ => {}
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::classic()
    }
}

/// The 16 message colors that color codes 0 to 15 select.
pub type Palette = [[u8; 3]; 16];

thread_local! {
    static ACTIVE: Cell<Theme> = Cell::new(Theme::classic());
    static PALETTE: Cell<Palette> = const { Cell::new(crate::colors::PALETTE) };
}

pub fn install(theme: Theme) {
    ACTIVE.set(theme);
}

pub fn install_palette(palette: Palette) {
    PALETTE.set(palette);
}

pub fn palette() -> Palette {
    PALETTE.get()
}

pub fn current() -> Theme {
    ACTIVE.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let color = Rgb::parse("d4d0c8").unwrap();
        assert_eq!(color, Rgb(212, 208, 200));
        assert_eq!(color.encode(), "D4D0C8");
        assert!(Rgb::parse("gg0000").is_none());
        assert!(Rgb::parse("fff").is_none());
    }

    #[test]
    fn named_slot_updates() {
        let mut theme = Theme::classic();
        theme.set("attention", Rgb(1, 2, 3));
        assert_eq!(theme.attention, Rgb(1, 2, 3));
        theme.set("nope", Rgb(9, 9, 9));
        assert_eq!(theme.face, Theme::classic().face);
    }
}
