// Copyright 2025 The Rustux Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

//! Dracula Theme for Rustica OS
//!
//! Default color scheme based on the Dracula theme.
//! Colors are configurable via constants below.

use uefi::proto::console::text::Color;

/// Dracula color palette
pub struct Theme {
    /// Background color
    pub background: Color,
    /// Current line/selection background
    pub background_alt: Color,
    /// Primary foreground text
    pub foreground: Color,
    /// User input text
    pub input: Color,
    /// Prompt color
    pub prompt: Color,
    /// System messages
    pub info: Color,
    /// Success messages
    pub success: Color,
    /// Warning messages
    pub warning: Color,
    /// Error messages
    pub error: Color,
}

/// Get the default Dracula theme
pub fn get_dracula_theme() -> Theme {
    // Dracula color palette (mapped to UEFI colors)
    // Background: #282a36 → Blue (closest UEFI match for dark bg)
    // Foreground: #f8f8f2 → White
    // Cyan: #8be9fd → Cyan
    // Green: #50fa7b → Green
    // Orange: #ffb86c → Yellow (UEFI doesn't have orange)
    // Pink: #ff79c6 → Magenta (UEFI doesn't have pink)
    // Purple: #bd93f9 → Magenta
    // Red: #ff5555 → Red
    // Yellow: #f1fa8c → Yellow

    Theme {
        // Dark background - using Blue as base (can be configured to Black if supported)
        background: Color::Blue,

        // Slightly lighter background for selections
        background_alt: Color::Black,

        // Primary foreground - white text
        foreground: Color::White,

        // User input - white for visibility
        input: Color::White,

        // Prompt - yellow (matches f1fa8c)
        prompt: Color::Yellow,

        // System info - cyan (matches 8be9fd)
        info: Color::Cyan,

        // Success - green (matches 50fa7b)
        success: Color::Green,

        // Warning - yellow (matches f1fa8c)
        warning: Color::Yellow,

        // Error - red (matches ff5555)
        error: Color::Red,
    }
}

/// Alternative theme configurations (can be selected at compile time)
pub mod themes {
    use super::*;

    /// Light theme alternative
    pub fn light_theme() -> Theme {
        Theme {
            background: Color::White,
            background_alt: Color::Black,
            foreground: Color::Black,
            input: Color::Black,
            prompt: Color::Blue,
            info: Color::Blue,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
        }
    }

    /// High contrast theme
    pub fn high_contrast_theme() -> Theme {
        Theme {
            background: Color::Black,
            background_alt: Color::Black,
            foreground: Color::White,
            input: Color::White,
            prompt: Color::Yellow,
            info: Color::Cyan,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
        }
    }
}

/// Select the active theme (configure this to change the default)
pub fn get_active_theme() -> Theme {
    // CONFIGURE DEFAULT THEME HERE
    // Options: get_dracula_theme(), themes::light_theme(), themes::high_contrast_theme()
    get_dracula_theme()
}
