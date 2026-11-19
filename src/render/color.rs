#[cfg(feature = "vello-renderer")]
use vello::peniko::Color;

/// Renderer-neutral 8-bit sRGBA color value shared across GPU (`vello`) and vector PDF (`krilla`) backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    #[inline]
    pub const fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    #[inline]
    pub const fn to_array(self) -> [u8; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// Parses a hex (`#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`) or named CSS/DT color string into `Rgba8`.
pub fn parse_color_rgba8(s: &str) -> Rgba8 {
    let s = s.trim();
    if s.is_empty() {
        return Rgba8::from_rgba8(0, 0, 0, 255);
    }

    if let Some(hex) = s.strip_prefix('#') {
        match hex.len() {
            // #RGB -> #RRGGBB
            3 => {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).unwrap_or(0);
                return Rgba8::from_rgba8(r, g, b, 255);
            }
            // #RGBA -> #RRGGBBAA
            4 => {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).unwrap_or(0);
                let a = u8::from_str_radix(&hex[3..4].repeat(2), 16).unwrap_or(255);
                return Rgba8::from_rgba8(r, g, b, a);
            }
            // #RRGGBB
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
                return Rgba8::from_rgba8(r, g, b, 255);
            }
            // #RRGGBBAA
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
                let a = u8::from_str_radix(&hex[6..8], 16).unwrap_or(255);
                return Rgba8::from_rgba8(r, g, b, a);
            }
            _ => return Rgba8::from_rgba8(0, 0, 0, 255),
        }
    }

    match s.to_ascii_lowercase().as_str() {
        "black" => Rgba8::from_rgba8(0, 0, 0, 255),
        "white" => Rgba8::from_rgba8(255, 255, 255, 255),
        "red" => Rgba8::from_rgba8(230, 50, 50, 255),
        "green" => Rgba8::from_rgba8(50, 180, 50, 255),
        "blue" => Rgba8::from_rgba8(50, 100, 230, 255),
        "yellow" => Rgba8::from_rgba8(240, 210, 50, 255),
        "orange" => Rgba8::from_rgba8(245, 130, 32, 255),
        "purple" => Rgba8::from_rgba8(145, 60, 200, 255),
        "cyan" => Rgba8::from_rgba8(40, 190, 210, 255),
        "gray" | "grey" => Rgba8::from_rgba8(128, 128, 128, 255),
        "lightgray" | "lightgrey" => Rgba8::from_rgba8(211, 211, 211, 255),
        "darkgray" | "darkgrey" => Rgba8::from_rgba8(60, 60, 60, 255),
        "transparent" => Rgba8::from_rgba8(0, 0, 0, 0),
        _ => Rgba8::from_rgba8(0, 0, 0, 255),
    }
}

/// Parse a hex or named color string into a `vello::peniko::Color`.
#[cfg(feature = "vello-renderer")]
pub fn parse_color(s: &str) -> Color {
    let c = parse_color_rgba8(s);
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Converts a `vello::peniko::Color` into `[u8; 4]` RGBA components.
#[cfg(feature = "vello-renderer")]
pub fn color_to_rgba8(c: &Color) -> [u8; 4] {
    let r = (c.components[0] * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (c.components[1] * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (c.components[2] * 255.0).round().clamp(0.0, 255.0) as u8;
    let a = (c.components[3] * 255.0).round().clamp(0.0, 255.0) as u8;
    [r, g, b, a]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_colors() {
        assert_eq!(parse_color_rgba8("#333"), Rgba8::from_rgba8(0x33, 0x33, 0x33, 255));
        assert_eq!(parse_color_rgba8("#1a2b3c"), Rgba8::from_rgba8(0x1a, 0x2b, 0x3c, 255));
        assert_eq!(parse_color_rgba8("#ff000080"), Rgba8::from_rgba8(0xff, 0x00, 0x00, 0x80));
    }

    #[test]
    fn test_named_colors() {
        assert_eq!(parse_color_rgba8("white"), Rgba8::from_rgba8(255, 255, 255, 255));
        assert_eq!(parse_color_rgba8("transparent"), Rgba8::from_rgba8(0, 0, 0, 0));
    }
}
