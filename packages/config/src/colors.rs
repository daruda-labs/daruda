use serde::{Deserialize, Serialize};
use std::fmt;

/// An RGB color parsed from a `#RRGGBB` hex string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl HexColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Convert to a `(u8, u8, u8)` tuple.
    pub const fn to_tuple(self) -> (u8, u8, u8) {
        (self.r, self.g, self.b)
    }
}

impl fmt::Display for HexColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl<'de> Deserialize<'de> for HexColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        parse_hex_color(&s).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "invalid hex color \"{s}\": expected #RRGGBB format"
            ))
        })
    }
}

impl Serialize for HexColor {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

fn parse_hex_color(s: &str) -> Option<HexColor> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(HexColor::new(r, g, b))
}

/// ANSI 8-color palette (indices 0–7 or 8–15).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct AnsiPalette {
    pub black: HexColor,
    pub red: HexColor,
    pub green: HexColor,
    pub yellow: HexColor,
    pub blue: HexColor,
    pub magenta: HexColor,
    pub cyan: HexColor,
    pub white: HexColor,
}

impl AnsiPalette {
    /// Return palette entries as an array indexed 0–7.
    pub fn as_array(&self) -> [HexColor; 8] {
        [
            self.black,
            self.red,
            self.green,
            self.yellow,
            self.blue,
            self.magenta,
            self.cyan,
            self.white,
        ]
    }
}

/// Default normal palette — matches xterm defaults.
impl Default for AnsiPalette {
    fn default() -> Self {
        Self {
            black: HexColor::new(0x00, 0x00, 0x00),
            red: HexColor::new(0xCC, 0x00, 0x00),
            green: HexColor::new(0x4E, 0x9A, 0x06),
            yellow: HexColor::new(0xC4, 0xA0, 0x00),
            blue: HexColor::new(0x34, 0x65, 0xA4),
            magenta: HexColor::new(0x75, 0x50, 0x7B),
            cyan: HexColor::new(0x06, 0x98, 0x9A),
            white: HexColor::new(0xD3, 0xD7, 0xCF),
        }
    }
}

/// Full color configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ColorConfig {
    /// Default foreground color.
    pub foreground: HexColor,
    /// Default background color.
    pub background: HexColor,
    /// Normal-intensity ANSI colors (indices 0–7).
    pub normal: AnsiPalette,
    /// Bright ANSI colors (indices 8–15).
    pub bright: AnsiPalette,
}

impl ColorConfig {
    /// Convert the 16-entry ANSI palette to a `[[u8; 3]; 16]` array suitable
    /// for passing to `ghostty_vt`.  Indices 0–7 are the normal palette;
    /// indices 8–15 are the bright palette.
    pub fn to_ansi_palette(&self) -> [[u8; 3]; 16] {
        let mut pal = [[0u8; 3]; 16];
        for (i, c) in self.normal.as_array().iter().enumerate() {
            pal[i] = [c.r, c.g, c.b];
        }
        for (i, c) in self.bright.as_array().iter().enumerate() {
            pal[8 + i] = [c.r, c.g, c.b];
        }
        pal
    }
}

impl Default for ColorConfig {
    fn default() -> Self {
        Self {
            foreground: HexColor::new(0xD4, 0xD4, 0xD4),
            background: HexColor::new(0x1E, 0x1E, 0x1E),
            normal: AnsiPalette::default(),
            bright: AnsiPalette {
                black: HexColor::new(0x55, 0x57, 0x53),
                red: HexColor::new(0xEF, 0x29, 0x29),
                green: HexColor::new(0x8A, 0xE2, 0x34),
                yellow: HexColor::new(0xFC, 0xE9, 0x4F),
                blue: HexColor::new(0x72, 0x9F, 0xCF),
                magenta: HexColor::new(0xAD, 0x7F, 0xA8),
                cyan: HexColor::new(0x34, 0xE2, 0xE2),
                white: HexColor::new(0xEE, 0xEE, 0xEC),
            },
        }
    }
}
