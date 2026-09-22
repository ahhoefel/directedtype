use vello::peniko::Color;

/// Parse a hex or named color string into a `vello::peniko::Color`.
pub fn parse_color(s: &str) -> Color {
    let s = s.trim();
    if s.is_empty() {
        return Color::from_rgba8(0, 0, 0, 255);
    }

    if let Some(hex) = s.strip_prefix('#') {
        match hex.len() {
            // #RGB -> #RRGGBB
            3 => {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).unwrap_or(0);
                return Color::from_rgba8(r, g, b, 255);
            }
            // #RGBA -> #RRGGBBAA
            4 => {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).unwrap_or(0);
                let a = u8::from_str_radix(&hex[3..4].repeat(2), 16).unwrap_or(255);
                return Color::from_rgba8(r, g, b, a);
            }
            // #RRGGBB
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
                return Color::from_rgba8(r, g, b, 255);
            }
            // #RRGGBBAA
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
                let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
                let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
                let a = u8::from_str_radix(&hex[6..8], 16).unwrap_or(255);
                return Color::from_rgba8(r, g, b, a);
            }
            _ => return Color::from_rgba8(0, 0, 0, 255),
        }
    }

    match s.to_ascii_lowercase().as_str() {
        "black" => Color::from_rgba8(0, 0, 0, 255),
        "white" => Color::from_rgba8(255, 255, 255, 255),
        "red" => Color::from_rgba8(230, 50, 50, 255),
        "green" => Color::from_rgba8(50, 180, 50, 255),
        "blue" => Color::from_rgba8(50, 100, 230, 255),
        "yellow" => Color::from_rgba8(240, 210, 50, 255),
        "orange" => Color::from_rgba8(245, 130, 32, 255),
        "purple" => Color::from_rgba8(145, 60, 200, 255),
        "cyan" => Color::from_rgba8(40, 190, 210, 255),
        "gray" | "grey" => Color::from_rgba8(128, 128, 128, 255),
        "lightgray" | "lightgrey" => Color::from_rgba8(211, 211, 211, 255),
        "darkgray" | "darkgrey" => Color::from_rgba8(60, 60, 60, 255),
        "transparent" => Color::from_rgba8(0, 0, 0, 0),
        _ => Color::from_rgba8(0, 0, 0, 255),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex_colors() {
        assert_eq!(parse_color("#333"), Color::from_rgba8(0x33, 0x33, 0x33, 255));
        assert_eq!(parse_color("#1a2b3c"), Color::from_rgba8(0x1a, 0x2b, 0x3c, 255));
        assert_eq!(parse_color("#ff000080"), Color::from_rgba8(0xff, 0x00, 0x00, 0x80));
    }

    #[test]
    fn test_named_colors() {
        assert_eq!(parse_color("white"), Color::from_rgba8(255, 255, 255, 255));
        assert_eq!(parse_color("transparent"), Color::from_rgba8(0, 0, 0, 0));
    }
}
