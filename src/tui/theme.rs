use ratatui::style::Color;
use std::sync::atomic::{AtomicU8, Ordering};

/// Color palette for a theme variant
pub struct Palette {
    pub fg: Color,
    pub red: Color,
    pub orange: Color,
    pub yellow: Color,
    pub green: Color,
    pub cyan: Color,
    pub purple: Color,
    pub dim: Color,
    pub comment: Color,
}

/// Available themes (index corresponds to palette array)
pub const THEMES: &[&str] = &["ember", "monokai", "nord", "solarized", "gruvbox", "everforest", "catppuccin"];

const PALETTES: &[Palette] = &[
    // Ember
    Palette {
        fg: Color::Rgb(0xB3, 0xB3, 0xB3),
        red: Color::Rgb(0xF0, 0x71, 0x73),
        orange: Color::Rgb(0xE6, 0x98, 0x75),
        yellow: Color::Rgb(0xE2, 0xAE, 0x6A),
        green: Color::Rgb(0x99, 0xC9, 0x83),
        cyan: Color::Rgb(0x78, 0xBD, 0xB4),
        purple: Color::Rgb(0xD8, 0x75, 0x95),
        dim: Color::Rgb(0x99, 0x99, 0x99),
        comment: Color::Rgb(0x5A, 0x5A, 0x5A),
    },
    // Monokai Pro Spectrum
    Palette {
        fg: Color::Rgb(0xFC, 0xFC, 0xFA),
        red: Color::Rgb(0xFF, 0x61, 0x88),
        orange: Color::Rgb(0xFC, 0x98, 0x67),
        yellow: Color::Rgb(0xFF, 0xD8, 0x66),
        green: Color::Rgb(0xA9, 0xDC, 0x76),
        cyan: Color::Rgb(0x78, 0xDC, 0xE8),
        purple: Color::Rgb(0xAB, 0x9D, 0xF2),
        dim: Color::Rgb(0x93, 0x92, 0x93),
        comment: Color::Rgb(0x72, 0x70, 0x72),
    },
    // Nord
    Palette {
        fg: Color::Rgb(0xD8, 0xDE, 0xE9),
        red: Color::Rgb(0xBF, 0x61, 0x6A),
        orange: Color::Rgb(0xD0, 0x87, 0x70),
        yellow: Color::Rgb(0xEB, 0xCB, 0x8B),
        green: Color::Rgb(0xA3, 0xBE, 0x8C),
        cyan: Color::Rgb(0x88, 0xC0, 0xD0),
        purple: Color::Rgb(0xB4, 0x8E, 0xAD),
        dim: Color::Rgb(0x61, 0x6E, 0x88),
        comment: Color::Rgb(0x4C, 0x56, 0x6A),
    },
    // Solarized Dark
    Palette {
        fg: Color::Rgb(0x93, 0xA1, 0xA1),
        red: Color::Rgb(0xDC, 0x32, 0x2F),
        orange: Color::Rgb(0xCB, 0x4B, 0x16),
        yellow: Color::Rgb(0xB5, 0x89, 0x00),
        green: Color::Rgb(0x85, 0x99, 0x00),
        cyan: Color::Rgb(0x2A, 0xA1, 0x98),
        purple: Color::Rgb(0x6C, 0x71, 0xC4),
        dim: Color::Rgb(0x58, 0x6E, 0x75),
        comment: Color::Rgb(0x65, 0x7B, 0x83),
    },
    // Gruvbox Dark
    Palette {
        fg: Color::Rgb(0xEB, 0xDB, 0xB2),
        red: Color::Rgb(0xFB, 0x49, 0x34),
        orange: Color::Rgb(0xFE, 0x80, 0x19),
        yellow: Color::Rgb(0xFA, 0xBD, 0x2F),
        green: Color::Rgb(0xB8, 0xBB, 0x26),
        cyan: Color::Rgb(0x8E, 0xC0, 0x7C),
        purple: Color::Rgb(0xD3, 0x86, 0x9B),
        dim: Color::Rgb(0x7C, 0x6F, 0x64),
        comment: Color::Rgb(0x92, 0x83, 0x74),
    },
    // Everforest Dark
    Palette {
        fg: Color::Rgb(0xD3, 0xC6, 0xAA),
        red: Color::Rgb(0xE6, 0x7E, 0x80),
        orange: Color::Rgb(0xE6, 0x98, 0x75),
        yellow: Color::Rgb(0xDB, 0xBC, 0x7F),
        green: Color::Rgb(0xA7, 0xC0, 0x80),
        cyan: Color::Rgb(0x7F, 0xBB, 0xB3),
        purple: Color::Rgb(0xD6, 0x99, 0xB6),
        dim: Color::Rgb(0x85, 0x92, 0x89),
        comment: Color::Rgb(0x7A, 0x84, 0x78),
    },
    // Catppuccin Mocha
    Palette {
        fg: Color::Rgb(0xCD, 0xD6, 0xF4),
        red: Color::Rgb(0xF3, 0x8B, 0xA8),
        orange: Color::Rgb(0xFA, 0xB3, 0x87),
        yellow: Color::Rgb(0xF9, 0xE2, 0xAF),
        green: Color::Rgb(0xA6, 0xE3, 0xA1),
        cyan: Color::Rgb(0x89, 0xDC, 0xEB),
        purple: Color::Rgb(0xCB, 0xA6, 0xF7),
        dim: Color::Rgb(0x6C, 0x70, 0x86),
        comment: Color::Rgb(0x58, 0x5B, 0x70),
    },
];

/// Active palette index (0 = ember default)
static ACTIVE: AtomicU8 = AtomicU8::new(0);

/// Get the currently active palette
pub fn current() -> &'static Palette {
    let idx = ACTIVE.load(Ordering::Relaxed) as usize;
    &PALETTES[idx.min(PALETTES.len() - 1)]
}

/// Set the active theme by name. Returns false if name is unknown.
pub fn set_theme(name: &str) -> bool {
    if let Some(idx) = THEMES.iter().position(|&t| t == name) {
        ACTIVE.store(idx as u8, Ordering::Relaxed);
        true
    } else {
        false
    }
}

/// Get the current theme name
pub fn current_theme() -> &'static str {
    let idx = ACTIVE.load(Ordering::Relaxed) as usize;
    THEMES[idx.min(THEMES.len() - 1)]
}
